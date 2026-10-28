//! User-triggered, local file ingestion. No provider, transport or AI dispatch.
mod csv;
use crate::{config, database_error, invalidate_journals, lifecycle, network, resolve};
use kassiber_envelope::{dict, s, AppError};
use kassiber_pydecimal::{Context, Decimal, DecimalError, Rounding};
use kassiber_pyjson::Value as Py;
use kassiber_store::rusqlite::{
    params, params_from_iter, types::Value as Sql, Connection, OptionalExtension,
};
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const CTX: Context = Context {
    prec: 32,
    rounding: Rounding::HalfEven,
};
type Fields = BTreeMap<String, Sql>;
#[derive(Debug)]
pub struct Request {
    pub profile_id: String,
    pub wallet: String,
    pub file: String,
    pub phoenix: bool,
}
/// Explicit entropy/clock injection for parity tests, never sourced from
/// environment variables in a production build.
pub struct Identity {
    pub ids: Vec<String>,
    pub timestamp: String,
}
struct Ids<'a> {
    identity: Option<&'a Identity>,
    at: usize,
}
impl Ids<'_> {
    fn next(&mut self, conn: &Connection) -> Result<String, AppError> {
        if let Some(identity) = self.identity {
            let id = identity
                .ids
                .get(self.at)
                .ok_or_else(|| AppError::new("internal_error", "Import test identities exhausted"))?
                .clone();
            self.at += 1;
            Ok(id)
        } else {
            lifecycle::uuid(conn)
        }
    }
    fn time(&self, conn: &Connection) -> Result<String, AppError> {
        self.identity
            .map_or_else(|| lifecycle::now(conn), |i| Ok(i.timestamp.clone()))
    }
}
fn decimal_error(e: DecimalError) -> AppError {
    let name = match e {
        DecimalError::Overflow => "Overflow",
        DecimalError::DivisionByZero => "DivisionByZero",
        _ => "InvalidOperation",
    };
    AppError::new("internal_error", format!("[<class 'decimal.{name}'>]"))
}
fn text(row: &Json, key: &str) -> String {
    row.get(key)
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn optional(row: &Json, key: &str) -> Option<String> {
    let text = text(row, key);
    let text = kassiber_pytext::strip(&text);
    (!text.is_empty()).then(|| text.to_owned())
}
fn dec(value: &str) -> Result<Decimal, AppError> {
    kassiber_pytext::strip(value.if_empty("0"))
        .parse()
        .map_err(|_| AppError::new("app_error", format!("Invalid decimal value: {value}")))
}
enum PriceNumber {
    Finite(Decimal),
    Special(String),
}
impl PriceNumber {
    fn parse(value: &str) -> Result<Self, AppError> {
        match special(value) {
            Some(v) => Ok(Self::Special(v)),
            None => dec(value).map(Self::Finite),
        }
    }
    fn multiply(&self, amount: &Decimal) -> Result<Self, AppError> {
        match self {
            Self::Finite(v) => amount.mul(v, &CTX).map(Self::Finite).map_err(decimal_error),
            Self::Special(v)
                if v.contains("sNaN") || v.contains("Infinity") && amount.is_zero() =>
            {
                Err(decimal_error(DecimalError::InvalidOperation))
            }
            Self::Special(v) => Ok(Self::Special(v.clone())),
        }
    }
    fn exact(&self) -> Result<String, AppError> {
        match self {
            Self::Finite(v) => v.to_fixed_string().map_err(decimal_error),
            Self::Special(v) => Ok(v.clone()),
        }
    }
    fn float(&self) -> Result<Sql, AppError> {
        let exact = self.exact()?;
        if exact.contains("sNaN") {
            return Err(AppError::new(
                "internal_error",
                "cannot convert signaling NaN to float",
            ));
        }
        // SQLite binds a quiet NaN as NULL, while the exact spelling remains.
        if exact.contains("NaN") {
            return Ok(Sql::Null);
        }
        Ok(Sql::Real(kassiber_pytext::parse_float(&exact).unwrap_or(
            if exact.starts_with('-') {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            },
        )))
    }
}
fn special(input: &str) -> Option<String> {
    let raw = kassiber_pytext::strip(input).replace('_', "");
    let negative = raw.starts_with('-');
    let raw = raw.trim_start_matches(['+', '-']);
    let lower = raw.to_ascii_lowercase();
    let sign = if negative { "-" } else { "" };
    if ["inf", "infinity"].contains(&lower.as_str()) {
        return Some(format!("{sign}Infinity"));
    }
    let (prefix, digits) = if let Some(digits) = lower.strip_prefix("snan") {
        ("sNaN", digits)
    } else {
        ("NaN", lower.strip_prefix("nan")?)
    };
    let digits = digits
        .chars()
        .map(|c| kassiber_pytext::decimal_value(u32::from(c)).map(|n| char::from(b'0' + n as u8)))
        .collect::<Option<String>>()?;
    Some(format!("{sign}{prefix}{}", digits.trim_start_matches('0')))
}
trait Empty {
    fn if_empty<'a>(&'a self, fallback: &'a str) -> &'a str;
}
impl Empty for str {
    fn if_empty<'a>(&'a self, fallback: &'a str) -> &'a str {
        if self.is_empty() {
            fallback
        } else {
            self
        }
    }
}
fn msat(d: &Decimal) -> Result<i64, AppError> {
    let d = d.mul(&dec("100000000000")?, &CTX).map_err(decimal_error)?;
    let result = d
        .quantize(
            0,
            &Context {
                prec: 1000,
                rounding: Rounding::HalfUp,
            },
        )
        .map_err(decimal_error)?
        .to_fixed_string()
        .map_err(decimal_error)?;
    result.parse().map_err(|_| {
        AppError::new(
            "internal_error",
            "Python int too large to convert to SQLite INTEGER",
        )
    })
}
fn sql(value: Option<String>) -> Sql {
    value.map_or(Sql::Null, Sql::Text)
}
fn string(fields: &Fields, key: &str) -> String {
    match fields.get(key) {
        Some(Sql::Text(v)) => v.clone(),
        _ => String::new(),
    }
}
fn truthy(value: Option<&Sql>) -> bool {
    match value {
        None | Some(Sql::Null) => false,
        Some(Sql::Text(s)) => !s.is_empty(),
        Some(Sql::Integer(n)) => *n != 0,
        Some(Sql::Real(n)) => *n != 0.,
        Some(Sql::Blob(v)) => !v.is_empty(),
    }
}
fn hash(value: &str) -> Option<String> {
    let text = kassiber_pytext::strip(value).to_lowercase();
    let mut half = false;
    let valid = text.bytes().all(|b| {
        if b.is_ascii_hexdigit() {
            half = !half;
            true
        } else {
            !half && b.is_ascii_whitespace()
        }
    });
    (text.len() == 64 && valid && !half).then_some(text)
}
fn asset(value: &str) -> String {
    let value = kassiber_pytext::strip(value);
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        value.to_lowercase()
    } else if ["l-btc", "lbtc"].contains(&value.to_lowercase().as_str()) {
        "LBTC".into()
    } else {
        value.to_uppercase()
    }
}
fn timestamp(row: &Json, keys: &[&str], required: bool) -> Result<Option<String>, AppError> {
    let value = keys
        .iter()
        .map(|key| text(row, key))
        .find(|v| !v.is_empty());
    match value {
        Some(v) => crate::time::timestamp(&v).map(Some),
        None if required => Err(AppError::new("app_error", "Missing occurred_at/date value")),
        None => Ok(None),
    }
}
fn priority(kind: &str, legacy: &str) -> i32 {
    match kind {
        "fmv_provider" => 10,
        "manual_rate_cache" => 20,
        "generic_import" => 30,
        "wallet_export" => 40,
        "btcpay_wallet_export" => 45,
        "exchange_execution" => 70,
        "btcpay_invoice" | "btcpay_payment" | "btcpay_payout" => 80,
        "manual_override" => 100,
        _ => match legacy {
            "manual" => 100,
            "rates_cache" => 10,
            "import" => 30,
            _ => 0,
        },
    }
}
const PRICE: &[&str] = &[
    "fiat_rate",
    "fiat_value",
    "fiat_rate_exact",
    "fiat_value_exact",
    "fiat_price_source",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_quality",
];
struct Record {
    fields: Fields,
    amount: Decimal,
    fee: Decimal,
}
fn normalize(input: &csv::Row) -> Result<Record, AppError> {
    let row = &input.data;
    let amount_special = special(&text(row, "amount"));
    let amount = if amount_special.is_some() {
        dec("0")?
    } else {
        dec(&text(row, "amount"))?
    };
    let direction = text(row, "direction");
    let direction = if direction.is_empty() {
        if amount_special.as_ref().is_some_and(|v| v.contains("NaN")) {
            return Err(decimal_error(DecimalError::InvalidOperation));
        }
        if amount.is_negative() {
            "outbound"
        } else {
            "inbound"
        }
    } else {
        match kassiber_pytext::strip(&direction).to_lowercase().as_str() {
            "in" | "inbound" | "receive" | "received" | "deposit" | "credit" | "buy" => "inbound",
            "out" | "outbound" | "send" | "sent" | "withdrawal" | "withdraw" | "debit" | "sell" => {
                "outbound"
            }
            _ => {
                return Err(AppError::new(
                    "app_error",
                    format!("Unsupported direction '{direction}'"),
                ))
            }
        }
    };
    let amount = amount.abs(&CTX).map_err(decimal_error)?;
    let fee_special = special(&text(row, "fee"));
    if amount_special
        .as_ref()
        .into_iter()
        .chain(fee_special.as_ref())
        .any(|s| s.contains("sNaN"))
    {
        return Err(decimal_error(DecimalError::InvalidOperation));
    }
    let fee = if fee_special.is_some() {
        dec("0")?
    } else {
        dec(&text(row, "fee"))?.abs(&CTX).map_err(decimal_error)?
    };
    let rate = (!text(row, "fiat_rate").is_empty())
        .then(|| PriceNumber::parse(&text(row, "fiat_rate")))
        .transpose()?;
    let value = (!text(row, "fiat_value").is_empty())
        .then(|| PriceNumber::parse(&text(row, "fiat_value")))
        .transpose()?;
    let value = match value {
        Some(v) => Some(v),
        None => rate.as_ref().map(|r| r.multiply(&amount)).transpose()?,
    };
    let priced = rate.is_some() || value.is_some();
    if input.rest.is_some()
        && row.get("raw_json").is_none_or(Json::is_null)
        && !row.as_object().unwrap().is_empty()
    {
        return Err(AppError::new(
            "internal_error",
            "'<' not supported between instances of 'NoneType' and 'str'",
        ));
    }
    let mut raw = if let Some(raw) = row.get("raw_json").filter(|v| !v.is_null()) {
        raw.clone()
    } else {
        let mut raw = row.clone();
        if let Some(rest) = &input.rest {
            raw.as_object_mut()
                .unwrap()
                .insert("null".into(), json!(rest));
        }
        raw
    };
    if let Some(s) = raw.as_str() {
        if let Ok(parsed) = serde_json::from_str(s) {
            raw = parsed;
        }
    }
    if let Some(object) = raw.as_object_mut() {
        object.remove("_kassiber_provenance");
    }
    let raw_json = if raw.is_object() || !raw.is_string() {
        config::stored_json(&raw)
    } else {
        raw.as_str().unwrap().to_owned()
    };
    let occurred = timestamp(row, &["occurred_at", "timestamp", "date"], true)?.unwrap();
    let confirmed = timestamp(row, &["confirmed_at"], false)?;
    let pricing_timestamp = timestamp(row, &["pricing_timestamp"], false)?
        .or_else(|| priced.then(|| confirmed.clone().unwrap_or_else(|| occurred.clone())));
    let kind = if priced {
        optional(row, "pricing_source_kind")
            .or_else(|| optional(row, "fiat_price_source_kind"))
            .unwrap_or_else(|| "generic_import".into())
            .to_lowercase()
    } else {
        String::new()
    };
    let quality = if priced {
        optional(row, "pricing_quality")
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| {
                if kind == "generic_import" {
                    "provider_sample".into()
                } else {
                    "exact".into()
                }
            })
    } else {
        "missing".into()
    };
    let mut fields = Fields::new();
    fields.insert(
        "external_id".into(),
        Sql::Text(text(row, "txid").if_empty(&text(row, "id")).to_owned()),
    );
    let native_id = hash(&string(&fields, "external_id"));
    let txid = native_id.as_ref().is_some_and(|id| {
        let declared = text(row, "external_id_kind")
            .if_empty(&text(&raw, "external_id_kind"))
            .trim()
            .to_lowercase();
        ["txid", "transaction_hash", "onchain_txid"].contains(&declared.as_str())
            || [row, &raw].iter().any(|payload| {
                let mut payloads = vec![*payload];
                for k in ["tx", "ownership_graph"] {
                    if let Some(v) = payload.get(k) {
                        payloads.push(v);
                    }
                }
                payloads.iter().any(|p| {
                    p.as_object().is_some_and(|o| {
                        o.iter().any(|(k, v)| {
                            let key: String = k
                                .to_lowercase()
                                .chars()
                                .filter(|c| c.is_alphanumeric())
                                .collect();
                            [
                                "txid",
                                "txhash",
                                "transactionid",
                                "transactionhash",
                                "onchaintxid",
                            ]
                            .contains(&key.as_str())
                                && v.as_str().and_then(hash).as_ref() == Some(id)
                        })
                    })
                })
            })
    });
    fields.insert("external_id_kind".into(), sql(txid.then(|| "txid".into())));
    fields.insert("occurred_at".into(), Sql::Text(occurred));
    fields.insert("confirmed_at".into(), sql(confirmed));
    fields.insert("direction".into(), Sql::Text(direction.into()));
    fields.insert(
        "asset".into(),
        Sql::Text(asset(text(row, "asset").if_empty("BTC"))),
    );
    if let Some(v) = amount_special.or(fee_special) {
        return Err(AppError::new(
            "internal_error",
            if v.contains("NaN") {
                "cannot convert NaN to integer"
            } else {
                "cannot convert Infinity to integer"
            },
        ));
    }
    fields.insert("amount".into(), Sql::Integer(msat(&amount)?));
    fields.insert("fee".into(), Sql::Integer(msat(&fee)?));
    fields.insert(
        "amount_includes_fee".into(),
        Sql::Integer(i64::from(!text(row, "amount_includes_fee").is_empty())),
    );
    fields.insert(
        "fiat_currency".into(),
        Sql::Text(
            optional(row, "fiat_currency")
                .unwrap_or_default()
                .to_uppercase(),
        ),
    );
    for (key, v) in [("fiat_rate", rate), ("fiat_value", value)] {
        let exact = v.as_ref().map(PriceNumber::exact).transpose()?;
        fields.insert(
            key.into(),
            v.as_ref()
                .map(PriceNumber::float)
                .transpose()?
                .unwrap_or(Sql::Null),
        );
        fields.insert(format!("{key}_exact"), sql(exact));
    }
    fields.insert(
        "fiat_price_source".into(),
        sql(priced.then(|| {
            match kind.as_str() {
                "fmv_provider" | "manual_rate_cache" => "rates_cache",
                "manual_override" => "manual",
                _ => "import",
            }
            .into()
        })),
    );
    fields.insert("pricing_source_kind".into(), sql(priced.then_some(kind)));
    fields.insert("pricing_quality".into(), Sql::Text(quality));
    fields.insert("pricing_timestamp".into(), sql(pricing_timestamp));
    fields.insert(
        "pricing_fetched_at".into(),
        sql(timestamp(row, &["pricing_fetched_at"], false)?),
    );
    for (key, alias) in [
        ("pricing_provider", "provider"),
        ("pricing_pair", "pair"),
        ("pricing_granularity", "granularity"),
        ("pricing_method", "method"),
        ("pricing_external_ref", "external_ref"),
    ] {
        fields.insert(
            key.into(),
            sql(optional(row, key).or_else(|| optional(row, alias))),
        );
    }
    for key in ["kind", "description", "counterparty"] {
        fields.insert(
            key.into(),
            row.get(key)
                .and_then(Json::as_str)
                .map_or(Sql::Null, |v| Sql::Text(v.into())),
        );
    }
    let mut boundary = None;
    for p in [row, &raw] {
        for key in [
            "privacy_boundary",
            "privacy_boundary_kind",
            "privacyBoundary",
            "privacyBoundaryKind",
            "privacy_hop",
            "privacy_hop_kind",
            "privacyHop",
            "privacyHopKind",
        ] {
            if let Some(value) = optional(p, key) {
                let normalized = value.to_lowercase().replace('-', "_");
                if ![
                    "coinjoin",
                    "collaborative",
                    "payjoin",
                    "payment_in_coinjoin",
                    "sweep",
                ]
                .contains(&normalized.as_str())
                {
                    return Err(AppError::new("validation",format!("Unsupported privacy boundary '{value}'")).with_hint("Supported privacy boundaries: coinjoin, collaborative, payjoin, payment_in_coinjoin, sweep."));
                }
                boundary = Some(normalized);
                break;
            }
        }
        if boundary.is_none()
            && [
                "islikelycoinjoin",
                "is_likely_coinjoin",
                "isLikelyCoinJoin",
                "likely_coinjoin",
                "likelyCoinJoin",
            ]
            .iter()
            .any(|k| {
                ["1", "true", "yes", "y", "on"].contains(&text(p, k).trim().to_lowercase().as_str())
            })
        {
            boundary = Some("coinjoin".into());
        }
        if boundary.is_some() {
            break;
        }
    }
    fields.insert("privacy_boundary".into(), sql(boundary));
    let payment_hash = hash(&text(row, "payment_hash"));
    fields.insert(
        "payment_hash_source".into(),
        sql(payment_hash
            .as_ref()
            .and_then(|_| optional(row, "payment_hash_source"))),
    );
    fields.insert("payment_hash".into(), sql(payment_hash));
    let funding = hash(&text(row, "swap_refund_funding_txid"));
    let vout = funding
        .as_ref()
        .and_then(|_| kassiber_pytext::parse_int(&text(row, "swap_refund_funding_vout")).ok())
        .and_then(|n| n.to_string().parse::<i64>().ok())
        .filter(|n| *n >= 0);
    fields.insert("swap_refund_funding_txid".into(), sql(funding));
    fields.insert(
        "swap_refund_funding_vout".into(),
        vout.map_or(Sql::Null, Sql::Integer),
    );
    fields.insert("raw_json".into(), Sql::Text(raw_json));
    Ok(Record {
        fields,
        amount,
        fee,
    })
}
fn fingerprint(wallet: &str, r: &Record) -> String {
    let value = json!({"wallet_id":wallet,"external_id":string(&r.fields,"external_id"),"occurred_at":string(&r.fields,"occurred_at"),"direction":string(&r.fields,"direction"),"asset":string(&r.fields,"asset"),"amount":r.amount.to_string(),"fee":r.fee.to_string()});
    Sha256::digest(config::stored_json(&value).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn change(id: &str, wallet: &str, r: &Record, fields: Vec<String>) -> Py {
    dict([
        ("transaction_id", s(id)),
        ("external_id", s(string(&r.fields, "external_id"))),
        ("wallet", s(wallet)),
        ("asset", s(string(&r.fields, "asset"))),
        ("direction", s(string(&r.fields, "direction"))),
        (
            "amount_msat",
            match r.fields["amount"] {
                Sql::Integer(n) => Py::from(n),
                _ => unreachable!(),
            },
        ),
        (
            "changed_fields",
            Py::List(fields.into_iter().map(s).collect()),
        ),
        (
            "pricing_external_ref",
            match &r.fields["pricing_external_ref"] {
                Sql::Text(v) => s(v.as_str()),
                _ => Py::Null,
            },
        ),
    ])
}
fn find(conn: &Connection, wallet: &str, r: &Record, fp: &str) -> Result<Option<Fields>, AppError> {
    let mut statement=conn.prepare("SELECT *, (SELECT authority_version FROM chain_observation_provenance WHERE transaction_id=transactions.id) AS observation_authority_version, (SELECT graph_hash FROM chain_observation_provenance WHERE transaction_id=transactions.id) AS observation_graph_hash, (SELECT quantity_hash FROM chain_observation_provenance WHERE transaction_id=transactions.id) AS observation_quantity_hash FROM transactions WHERE fingerprint=? OR (wallet_id=? AND external_id=? AND direction=? AND asset=? AND amount=? AND fee=?) ORDER BY (fingerprint=?) DESC,created_at DESC LIMIT 1").map_err(database_error)?;
    let columns = statement
        .column_names()
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    statement
        .query_row(
            params![
                fp,
                wallet,
                string(&r.fields, "external_id"),
                r.fields["direction"],
                r.fields["asset"],
                r.fields["amount"],
                r.fields["fee"],
                fp
            ],
            |row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, k)| Ok((k.clone(), row.get(i)?)))
                    .collect()
            },
        )
        .optional()
        .map_err(database_error)
}
fn merge(existing: &Fields, r: &Record, fp: &str) -> Result<Fields, AppError> {
    let mut updates = Fields::new();
    let incoming = &r.fields;
    if string(incoming, "external_id_kind") == "txid"
        && string(existing, "external_id_kind") != "txid"
    {
        updates.insert("external_id_kind".into(), Sql::Text("txid".into()));
        if existing.get("external_id") != incoming.get("external_id") {
            updates.insert("external_id".into(), incoming["external_id"].clone());
        }
    }
    if string(existing, "occurred_at") == "1970-01-01T00:00:00Z"
        && string(incoming, "occurred_at") != "1970-01-01T00:00:00Z"
    {
        updates.insert("occurred_at".into(), incoming["occurred_at"].clone());
    }
    if string(existing, "fingerprint") != fp
        && updates
            .get("occurred_at")
            .unwrap_or(&existing["occurred_at"])
            == &incoming["occurred_at"]
    {
        updates.insert("fingerprint".into(), Sql::Text(fp.into()));
    }
    let confirmed = !truthy(existing.get("confirmed_at")) && truthy(incoming.get("confirmed_at"));
    if confirmed {
        updates.insert("confirmed_at".into(), incoming["confirmed_at"].clone());
    }
    let price = truthy(incoming.get("pricing_source_kind"));
    let had_price = [
        "fiat_rate",
        "fiat_value",
        "fiat_rate_exact",
        "fiat_value_exact",
    ]
    .iter()
    .any(|k| existing.get(*k).is_some_and(|v| v != &Sql::Null));
    if price
        && (!had_price
            || priority(&string(incoming, "pricing_source_kind"), "")
                >= priority(
                    &string(existing, "pricing_source_kind"),
                    &string(existing, "fiat_price_source"),
                ))
    {
        for key in PRICE {
            updates.insert((*key).into(), incoming[*key].clone());
        }
    } else if confirmed && string(existing, "fiat_price_source") == "rates_cache" {
        for key in PRICE {
            updates.insert((*key).into(), Sql::Null);
        }
    }
    for key in ["kind", "description", "counterparty", "privacy_boundary"] {
        if !truthy(existing.get(key)) && truthy(incoming.get(key)) {
            updates.insert(key.into(), incoming[key].clone());
        }
    }
    if !truthy(existing.get("payment_hash")) && truthy(incoming.get("payment_hash")) {
        for key in ["payment_hash", "payment_hash_source"] {
            updates.insert(key.into(), incoming[key].clone());
        }
    }
    if !truthy(existing.get("swap_refund_funding_txid"))
        && truthy(incoming.get("swap_refund_funding_txid"))
    {
        for key in ["swap_refund_funding_txid", "swap_refund_funding_vout"] {
            updates.insert(key.into(), incoming[key].clone());
        }
    }
    // A file cannot replace an observer's committed graph or chain legs.
    let raw: Json = serde_json::from_str(&string(existing, "raw_json")).unwrap_or(Json::Null);
    let raw_incoming: Json =
        serde_json::from_str(&string(incoming, "raw_json")).unwrap_or(Json::Null);
    let legs = |raw: &Json| {
        let graph = raw.get("tx").filter(|v| v.is_object()).unwrap_or(raw);
        ["vin", "vout"]
            .iter()
            .any(|k| graph.get(k).is_some_and(Json::is_array))
    };
    let stored: Json = Json::Object(
        existing
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    match v {
                        Sql::Integer(n) => json!(n),
                        Sql::Text(v) => json!(v),
                        Sql::Real(n) => json!(n),
                        _ => Json::Null,
                    },
                )
            })
            .collect(),
    );
    let committed = kassiber_custody::evidence::current_observation(&stored)
        .map_err(|e| AppError::new("internal_error", e))?;
    let graph_upgrade = graph_upgrade(&raw, &raw_incoming);
    if (!updates.is_empty() || graph_upgrade)
        && !committed
        && (graph_upgrade || !legs(&raw) || legs(&raw_incoming))
        && incoming["raw_json"] != existing["raw_json"]
    {
        updates.insert("raw_json".into(), incoming["raw_json"].clone());
    }
    Ok(updates)
}
fn graph_upgrade(old: &Json, new: &Json) -> bool {
    let version = |v: &Json| {
        v.get("ownership_graph_version")
            .and_then(Json::as_u64)
            .unwrap_or(0)
    };
    if version(new) > version(old) {
        return true;
    }
    if version(old) == 0 || version(new) != version(old) {
        return false;
    }
    fn evidence(raw: &Json) -> BTreeMap<String, Json> {
        let mut out = BTreeMap::new();
        for side in ["vin", "vout"] {
            for (position, item) in raw
                .get(side)
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let key = if side == "vin" {
                    json!([
                        text(item, "txid"),
                        item.get("vout").cloned().unwrap_or(json!(position))
                    ])
                } else {
                    item.get("n").cloned().unwrap_or(json!(position))
                };
                let item = if side == "vin" {
                    item.get("prevout").unwrap_or(&Json::Null)
                } else {
                    item
                };
                for field in ["scriptpubkey", "value_sats", "asset_id", "asset", "role"] {
                    if let Some(v) = item.get(field).filter(|v| !v.is_null() && *v != &json!("")) {
                        out.insert(format!("{side}:{key}:{field}"), v.clone());
                    }
                }
            }
        }
        out
    }
    let old = evidence(old);
    let new = evidence(new);
    old.iter().all(|(k, v)| {
        new.get(k)
            .is_some_and(|n| n == v || k.ends_with(":role") && v == "external" && n == "owned")
    }) && old != new
}
fn update(conn: &Connection, table: &str, id: &str, fields: &Fields) -> Result<(), AppError> {
    let assignments = fields
        .keys()
        .map(|k| format!("{k}=?"))
        .collect::<Vec<_>>()
        .join(",");
    let mut values = fields.values().cloned().collect::<Vec<_>>();
    values.push(Sql::Text(id.into()));
    conn.execute(
        &format!("UPDATE {table} SET {assignments} WHERE id=?"),
        params_from_iter(values),
    )
    .map_err(database_error)?;
    Ok(())
}
fn insert(conn: &Connection, table: &str, fields: &Fields) -> Result<(), AppError> {
    let columns = fields.keys().cloned().collect::<Vec<_>>().join(",");
    let placeholders = vec!["?"; fields.len()].join(",");
    conn.execute(
        &format!("INSERT INTO {table}({columns}) VALUES({placeholders})"),
        params_from_iter(fields.values()),
    )
    .map_err(database_error)?;
    Ok(())
}
pub fn execute(
    conn: &Connection,
    request: Request,
    identity: Option<&Identity>,
) -> Result<Py, AppError> {
    let wallet = resolve(conn, &request.profile_id, &request.wallet)?;
    let mut rows = csv::load(&request.file)?;
    if request.phoenix {
        rows = phoenix(rows)?;
    }
    let records = rows.iter().map(normalize).collect::<Result<Vec<_>, _>>()?;
    conn.execute_batch("SAVEPOINT wallet_file_import")
        .map_err(database_error)?;
    let result = apply(
        conn,
        &request,
        &wallet,
        &rows,
        &records,
        &mut Ids { identity, at: 0 },
    );
    if result.is_ok() {
        conn.execute_batch("RELEASE wallet_file_import")
            .map_err(database_error)?;
    } else {
        conn.execute_batch("ROLLBACK TO wallet_file_import;RELEASE wallet_file_import")
            .map_err(database_error)?;
    }
    result
}
fn apply(
    conn: &Connection,
    request: &Request,
    wallet: &crate::Wallet,
    rows: &[csv::Row],
    records: &[Record],
    ids: &mut Ids<'_>,
) -> Result<Py, AppError> {
    let profile = &request.profile_id;
    let (workspace, currency): (String, String) = conn
        .query_row(
            "SELECT workspace_id,fiat_currency FROM profiles WHERE id=?",
            [profile],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(database_error)?;
    let configuration: String = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id=?",
            [&wallet.id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let configuration = serde_json::from_str(&configuration).unwrap_or(Json::Null);
    for r in records {
        let raw = serde_json::from_str(&string(&r.fields, "raw_json")).unwrap_or(Json::Null);
        network::guard_import(
            conn,
            profile,
            &configuration,
            &raw,
            &string(&r.fields, "asset"),
        )?;
    }
    let mut imported = Vec::new();
    let mut updated = Vec::new();
    let mut unchanged = 0;
    let mut skipped = 0;
    let mut inserted_ids = Vec::new();
    for r in records {
        let fp = fingerprint(&wallet.id, r);
        let existing = find(conn, &wallet.id, r, &fp)?;
        let fiat = string(&r.fields, "fiat_currency");
        if !fiat.is_empty()
            && fiat != currency.to_uppercase()
            && truthy(r.fields.get("pricing_source_kind"))
        {
            return Err(AppError::new("validation",format!("Imported price currency {fiat} does not match profile fiat currency {}",currency.to_uppercase())).with_hint("Import the file into a profile with the same fiat currency, or remove the imported fiat price values and price the transactions separately."));
        }
        if let Some(existing) = existing {
            let id = string(&existing, "id");
            let changes = merge(&existing, r, &fp)?;
            if changes.is_empty() {
                unchanged += 1;
            } else {
                update(conn, "transactions", &id, &changes)?;
                updated.push(change(
                    &id,
                    &wallet.label,
                    r,
                    changes.keys().cloned().collect(),
                ));
            }
            skipped += 1;
        } else {
            let id = ids.next(conn)?;
            let mut fields = r.fields.clone();
            for (key, value) in [
                ("id", id.clone()),
                ("workspace_id", workspace.clone()),
                ("profile_id", profile.clone()),
                ("wallet_id", wallet.id.clone()),
                ("fingerprint", fp),
                ("created_at", ids.time(conn)?),
            ] {
                fields.insert(key.into(), Sql::Text(value));
            }
            if string(&fields, "external_id").is_empty() {
                fields.insert("external_id".into(), Sql::Null);
            }
            if fiat.is_empty() {
                fields.insert("fiat_currency".into(), Sql::Text(currency.clone()));
            }
            insert(conn, "transactions", &fields)?;
            imported.push(change(
                &id,
                &wallet.label,
                r,
                vec!["metadata".into(), "pricing".into(), "transaction".into()],
            ));
            inserted_ids.push(id);
        }
    }
    let invalidated = !imported.is_empty() || !updated.is_empty();
    if invalidated {
        invalidate_journals(conn, profile)?;
    }
    let format = if request.phoenix {
        "phoenix_csv"
    } else {
        "csv"
    };
    let mut result = vec![
        ("wallet", s(wallet.label.as_str())),
        ("source", s(format!("file:{format}"))),
        ("imported", Py::from(imported.len() as i64)),
        ("skipped", Py::from(skipped)),
        ("unchanged", Py::from(unchanged)),
        ("journal_invalidated", Py::Bool(invalidated)),
        ("inserted_records", Py::List(imported)),
        ("updated_records", Py::List(updated)),
    ];
    if request.phoenix {
        phoenix_metadata(
            conn,
            (profile, &workspace),
            &wallet.id,
            rows,
            ids,
            &mut result,
            invalidated,
        )?;
    }
    if !inserted_ids.is_empty() {
        let batch = ids.next(conn)?;
        conn.execute("INSERT INTO import_batches(id,workspace_id,profile_id,wallet_id,source_format,source_filename,column_map_json,imported_at,rows_inserted,rows_updated,rows_skipped) VALUES(?,?,?,?,?,?,NULL,?,?,0,?)",params![batch,workspace,profile,wallet.id,format,std::path::Path::new(&request.file).file_name().unwrap_or_default().to_string_lossy(),ids.time(conn)?,inserted_ids.len() as i64,skipped]).map_err(database_error)?;
        for id in inserted_ids {
            conn.execute("INSERT OR IGNORE INTO import_batch_transactions(batch_id,transaction_id) VALUES(?,?)",[&batch,&id]).map_err(database_error)?;
        }
        result.push(("import_batch_id", s(batch)));
    }
    result.push(("input_format", s(format)));
    let path = kassiber_paths::abspath(std::path::Path::new(&request.file))
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    result.push(("file", s(path.to_string_lossy().into_owned())));
    Ok(dict(result))
}
fn phoenix(_rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    Err(AppError::new("not_implemented", "Phoenix file importer"))
}
fn phoenix_metadata(
    _conn: &Connection,
    _scope: (&str, &str),
    _wallet: &str,
    _rows: &[csv::Row],
    _ids: &mut Ids<'_>,
    _result: &mut Vec<(&str, Py)>,
    _invalidated: bool,
) -> Result<(), AppError> {
    Ok(())
}

#[cfg(test)]
mod tests;
