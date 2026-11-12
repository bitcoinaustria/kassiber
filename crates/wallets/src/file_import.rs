//! User-triggered ingestion. Document OCR requires an exact local-provider grant.
pub mod analysis;
pub mod attachment;
mod binance;
mod btcpay;
mod bull;
mod bull_wallet;
mod coinfinity;
pub(crate) mod csv;
pub mod document;
pub mod document_chat;
mod exchange;
pub mod exchange_api;
mod generic_json;
pub mod ledger;
mod ledgerlive;
mod phoenix;
pub(crate) mod platform_csv;
mod pocket;
mod river;
pub mod samourai;
mod strike;
mod synthetic;
pub use synthetic::synthetic_csv_identity;
mod tax_platform;
mod twentyone;
mod wasabi;
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
    pub format: Format,
    pub mode: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Phoenix,
    River,
    Bull,
    BullWallet,
    Coinfinity,
    TwentyOne,
    Strike,
    Pocket,
    LedgerLive,
    CoinTracking,
    Blockpit,
    Binance,
    BtcpayCsv,
    BtcpayJson,
    Ledger,
    Wasabi,
    Document,
}
impl Format {
    /// The importer behind a desktop `source_format`, where one is native.
    pub fn from_source_format(name: &str) -> Option<Self> {
        Some(match name {
            "json" => Self::Json,
            "csv" => Self::Csv,
            "btcpay_json" => Self::BtcpayJson,
            "btcpay_csv" => Self::BtcpayCsv,
            "phoenix_csv" => Self::Phoenix,
            "river_csv" => Self::River,
            "bullbitcoin_csv" => Self::Bull,
            "bullbitcoin_wallet_csv" => Self::BullWallet,
            "coinfinity_csv" => Self::Coinfinity,
            "21bitcoin_csv" => Self::TwentyOne,
            "pocketbitcoin_csv" => Self::Pocket,
            "strike_csv" => Self::Strike,
            "ledgerlive_csv" => Self::LedgerLive,
            "binance_supplemental_csv" => Self::Binance,
            "cointracking_csv" => Self::CoinTracking,
            "blockpit_csv" => Self::Blockpit,
            "wasabi_bundle" => Self::Wasabi,
            "generic_ledger" => Self::Ledger,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::Document => "document_import",
            Self::Wasabi => "wasabi_bundle",
            Self::Csv => "csv",
            Self::BtcpayCsv => "btcpay_csv",
            Self::BtcpayJson => "btcpay_json",
            Self::Json => "json",
            Self::Phoenix => "phoenix_csv",
            Self::River => "river_csv",
            Self::Bull => "bullbitcoin_csv",
            Self::BullWallet => "bullbitcoin_wallet_csv",
            Self::Coinfinity => "coinfinity_csv",
            Self::TwentyOne => "21bitcoin_csv",
            Self::Strike => "strike_csv",
            Self::Pocket => "pocketbitcoin_csv",
            Self::LedgerLive => "ledgerlive_csv",
            Self::CoinTracking => "cointracking_csv",
            Self::Blockpit => "blockpit_csv",
            Self::Binance => "binance_supplemental_csv",
            Self::Ledger => "generic_ledger",
        }
    }
    fn load(self, file: &str) -> Result<Vec<csv::Row>, AppError> {
        match self {
            Self::Document => Err(AppError::new(
                "validation",
                "Document imports require reviewed draft rows",
            )),
            Self::CoinTracking | Self::Blockpit => tax_platform::load(file, self),
            Self::Wasabi => wasabi::load(file).and_then(|b| wasabi::rows(&b.records)),
            Self::Json => generic_json::load(file),
            Self::BtcpayCsv | Self::BtcpayJson => btcpay::load(file, self == Self::BtcpayJson),
            Self::Csv => csv::load(file),
            Self::Binance => binance::parse(csv::rows(&csv::read_sig(file)?)?),
            Self::Phoenix => phoenix::parse(csv::load(file)?),
            Self::River => river::parse(csv::load(file)?),
            Self::Bull => bull::parse(csv::load(file)?),
            Self::BullWallet => bull_wallet::parse(csv::load(file)?),
            Self::Coinfinity => coinfinity::parse(csv::rows(&csv::read_sig(file)?)?),
            Self::TwentyOne => twentyone::parse(csv::load(file)?),
            Self::Strike => strike::parse(csv::load(file)?),
            Self::Pocket => pocket::parse(csv::load(file)?),
            Self::LedgerLive => ledgerlive::parse(csv::rows(&csv::read_sig(file)?)?),
            Self::Ledger => ledger::load(file, None),
        }
    }
    fn evidence(self) -> bool {
        matches!(
            self,
            Self::Bull | Self::Coinfinity | Self::Pocket | Self::Binance
        )
    }
    fn migration(self) -> bool {
        matches!(self, Self::CoinTracking | Self::Blockpit)
    }
    fn migration_kind(self) -> &'static str {
        match self {
            Self::Blockpit => "blockpit",
            _ => "cointracking",
        }
    }
    fn active(self) -> bool {
        matches!(self, Self::TwentyOne | Self::Strike)
    }
    fn provider(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Binance => ("Binance", "binance", "binance_rows"),
            Self::Coinfinity => ("Coinfinity", "coinfinity", "coinfinity_rows"),
            Self::Pocket => ("Pocket Bitcoin", "pocketbitcoin", "pocketbitcoin_rows"),
            Self::TwentyOne => ("21bitcoin", "21bitcoin", "twentyonebitcoin_rows"),
            Self::Strike => ("Strike", "strike", "strike_rows"),
            _ => ("Bull Bitcoin", "bullbitcoin", "bullbitcoin_rows"),
        }
    }
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
#[derive(Clone)]
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
    fn finite(self, field: &str) -> Result<Self, AppError> {
        let text = match self {
            Self::Finite(value) => value.to_string(),
            Self::Special(text) => text,
        };
        kassiber_pydecimal::finite_price(&text)
            .map(Self::Finite)
            .map_err(|e| AppError::new("validation", e.message(field)))
    }
    fn import_price(value: &str, field: &str) -> Result<Self, AppError> {
        Self::parse(value)
            .map_err(|_| AppError::new("validation", format!("{field} must be a decimal number")))?
            .finite(field)
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
fn amount_is_zero(value: &PriceNumber) -> bool {
    matches!(value, PriceNumber::Finite(v) if v.is_zero())
}
fn special(input: &str) -> Option<String> {
    let raw = kassiber_pytext::strip(input).replace('_', "");
    let negative = raw.starts_with('-');
    let raw = raw
        .strip_prefix('+')
        .or_else(|| raw.strip_prefix('-'))
        .unwrap_or(&raw);
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
            !half && matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
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
        None if required
            || keys
                .iter()
                .any(|key| row.get(*key) == Some(&Json::Bool(false))) =>
        {
            Err(AppError::new("app_error", "Missing occurred_at/date value"))
        }
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
    fingerprint_identity: Option<String>,
    ignore_fee: bool,
    binding_errors: BTreeMap<String, AppError>,
    economics: bool,
    tolerance: i64,
    deferred: Option<AppError>,
}
impl Record {
    fn insert_binding_error(&self) -> Option<&AppError> {
        // Match the reference INSERT parameter order, rather than map order.
        [
            "external_id",
            "asset",
            "fiat_currency",
            "pricing_source_kind",
            "pricing_provider",
            "pricing_pair",
            "pricing_granularity",
            "pricing_method",
            "pricing_external_ref",
            "pricing_quality",
            "kind",
            "description",
            "counterparty",
            "raw_json",
            "payment_hash_source",
        ]
        .into_iter()
        .find_map(|key| self.binding_errors.get(key))
    }
}
fn normalize(input: &csv::Row) -> Result<Record, AppError> {
    normalize_labeled(input, "")
}
/// `normalize_import_record(record, source_label)`: an unpriced kind is
/// inferred from the source label when the row names none.
fn normalize_labeled(input: &csv::Row, label: &str) -> Result<Record, AppError> {
    if let Some(original) = &input.original {
        let prepared = generic_json::prepare(original)?;
        let mut record = normalize_labeled(&prepared, label)?;
        for key in ["kind", "description", "counterparty"] {
            match generic_json::metadata(original, key) {
                Ok(value) => {
                    record.fields.insert(key.into(), value);
                }
                Err(error) => {
                    let nonempty = kassiber_envelope::get(original, key)
                        .is_some_and(kassiber_envelope::truthy);
                    record.fields.insert(
                        key.into(),
                        Sql::Blob(if nonempty { vec![1] } else { vec![] }),
                    );
                    record.binding_errors.insert(key.into(), error);
                }
            }
        }
        record
            .binding_errors
            .extend(generic_json::text_bindings(original, &record.fields));
        return Ok(record);
    }
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
        .then(|| PriceNumber::import_price(&text(row, "fiat_rate"), "fiat_rate"))
        .transpose()?;
    let value = (!text(row, "fiat_value").is_empty())
        .then(|| PriceNumber::import_price(&text(row, "fiat_value"), "fiat_value"))
        .transpose()?;
    let value = match value {
        Some(v) => Some(v),
        None => rate
            .as_ref()
            .map(|r| {
                if let Some(special) = &amount_special {
                    if special.contains("Infinity") && amount_is_zero(r) {
                        return Err(decimal_error(DecimalError::InvalidOperation));
                    }
                    return Err(AppError::new(
                        "validation",
                        "fiat_value must be a finite decimal number",
                    ));
                }
                r.multiply(&amount)?.finite("fiat_value")
            })
            .transpose()?,
    };
    // A value-only price stays a total assertion: its unit rate is validated
    // but not stored, so a later amount correction cannot rescale it.
    match (&rate, &value) {
        (None, Some(_)) if amount_special.is_some() => {
            let special = amount_special.as_ref().unwrap();
            if special.contains("NaN") {
                return Err(AppError::new(
                    "validation",
                    "fiat_rate must be a finite decimal number",
                ));
            }
        }
        (None, Some(PriceNumber::Finite(value))) if !amount.is_zero() => {
            let rate = value.div(&amount, &CTX).map_err(decimal_error)?;
            PriceNumber::Finite(rate).finite("fiat_rate")?;
        }
        _ => (),
    }
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
    let supplied_raw = if let Some(raw) = row.get("raw_json").filter(|v| !v.is_null()) {
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
    let supplied_raw = supplied_raw
        .as_str()
        .map_or_else(|| config::stored_json(&supplied_raw), str::to_owned);
    let (raw_json, raw) = generic_json::raw(&Py::from(supplied_raw.as_str()))?;
    let (raw_json, raw) = generic_json::stamp_provenance(label, raw_json, raw)?;
    let occurred = timestamp(row, &["occurred_at", "timestamp", "date"], true)?.unwrap();
    let confirmed = timestamp(row, &["confirmed_at"], false)?;
    let pricing_timestamp = timestamp(row, &["pricing_timestamp"], false)?
        .or_else(|| priced.then(|| confirmed.clone().unwrap_or_else(|| occurred.clone())));
    let kind = if priced {
        let label = kassiber_pytext::lower(kassiber_pytext::strip(label));
        let inferred = if label.starts_with("btcpay") {
            "btcpay_wallet_export"
        } else if label.starts_with("phoenix") {
            "wallet_export"
        } else {
            "generic_import"
        };
        kassiber_pytext::strip(
            text(row, "pricing_source_kind")
                .if_empty(text(row, "fiat_price_source_kind").if_empty(inferred)),
        )
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
    let integer_error = |v: &String| {
        AppError::new(
            "internal_error",
            if v.contains("NaN") {
                "cannot convert NaN to integer"
            } else {
                "cannot convert Infinity to integer"
            },
        )
    };
    if let Some(v) = &amount_special {
        return Err(integer_error(v));
    }
    let fetched_at = timestamp(row, &["pricing_fetched_at"], false)?;
    // `pricing_payload(amount_msat=...)` bounds the valuation each price
    // implies for the row's quantity before anything is stored.
    let exact = |v: &Option<PriceNumber>| v.as_ref().map(PriceNumber::exact).transpose();
    kassiber_pydecimal::validate_transaction_price(
        "pricing input",
        &kassiber_pydecimal::btc_to_msat(&amount, &CTX).map_err(decimal_error)?,
        exact(&rate)?.as_deref(),
        exact(&value)?.as_deref(),
        &CTX,
    )
    .map_err(|error| match error {
        kassiber_pydecimal::TransactionPriceError::Validation(message) => {
            AppError::new("validation", message)
        }
        kassiber_pydecimal::TransactionPriceError::Arithmetic(error) => decimal_error(error),
    })?;
    // Python keeps a non-finite fee Decimal until the row's fingerprint
    // converts it to msat, after the network guard.
    let deferred = fee_special.as_ref().map(integer_error);
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
        sql((priced && !kind.is_empty()).then(|| {
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
    fields.insert("pricing_fetched_at".into(), sql(fetched_at));
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
    let boundary = privacy_boundary(row, &raw)?;
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
        fingerprint_identity: optional(row, "_tax_platform_row_identity"),
        ignore_fee: !text(row, "_match_existing_ignore_fee").is_empty(),
        binding_errors: BTreeMap::new(),
        economics: row.get("_exchange_evidence_match_by_economics") == Some(&Json::Bool(true)),
        tolerance: row
            .get("_exchange_evidence_match_time_tolerance_seconds")
            .and_then(Json::as_i64)
            .unwrap_or(0),
        deferred,
    })
}
/// `_validate_import_price_currency(profile, normalized)`.
fn price_currency(r: &Record, currency: &str) -> Result<(), AppError> {
    let fiat = string(&r.fields, "fiat_currency");
    if !fiat.is_empty()
        && fiat != currency.to_uppercase()
        && ["fiat_rate_exact", "fiat_value_exact"]
            .iter()
            .any(|key| r.fields.get(*key).is_some_and(|v| *v != Sql::Null))
    {
        return Err(AppError::new("validation",format!("Imported price currency {fiat} does not match profile fiat currency {}",currency.to_uppercase())).with_hint("Import the file into a profile with the same fiat currency, or remove the imported fiat price values and price the transactions separately."));
    }
    Ok(())
}
fn privacy_boundary(row: &Json, raw: &Json) -> Result<Option<String>, AppError> {
    for p in [row, raw] {
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
            if let Some(value) = p.get(key).filter(|v| !v.is_null()) {
                let value = kassiber_pytext::loads(&config::stored_json(value))
                    .map(|v| kassiber_envelope::py_str(&v))
                    .unwrap_or_default();
                let value = kassiber_pytext::strip(&value);
                if value.is_empty() {
                    continue;
                }
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
                return Ok(Some(normalized));
            }
        }
        if [
            "islikelycoinjoin",
            "is_likely_coinjoin",
            "isLikelyCoinJoin",
            "likely_coinjoin",
            "likelyCoinJoin",
        ]
        .iter()
        .any(|k| {
            p.get(k).is_some_and(|v| {
                v == &Json::Bool(true)
                    || v.as_i64() == Some(1)
                    || v.as_str().is_some_and(|v| {
                        ["1", "true", "yes", "on"]
                            .contains(&kassiber_pytext::strip(v).to_lowercase().as_str())
                    })
            })
        }) {
            return Ok(Some("coinjoin".into()));
        }
    }
    Ok(None)
}
fn fingerprint_id(r: &Record) -> String {
    let external = string(&r.fields, "external_id");
    if external.is_empty() {
        r.fingerprint_identity.clone().unwrap_or_default()
    } else {
        external
    }
}
fn fingerprint(wallet: &str, r: &Record) -> String {
    let value = json!({"wallet_id":wallet,"external_id":fingerprint_id(r),"occurred_at":string(&r.fields,"occurred_at"),"direction":string(&r.fields,"direction"),"asset":string(&r.fields,"asset"),"amount":r.amount.to_string(),"fee":r.fee.to_string()});
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
    let routing = string(&r.fields, "kind") == "routing_income";
    let relax = string(&r.fields, "pricing_source_kind") == "exchange_execution" || r.ignore_fee;
    let mut statement=conn.prepare("SELECT *, (SELECT authority_version FROM chain_observation_provenance WHERE transaction_id=transactions.id) AS observation_authority_version, (SELECT graph_hash FROM chain_observation_provenance WHERE transaction_id=transactions.id) AS observation_graph_hash, (SELECT quantity_hash FROM chain_observation_provenance WHERE transaction_id=transactions.id) AS observation_quantity_hash FROM transactions WHERE fingerprint=? OR (wallet_id=? AND external_id=? AND ?!='' AND direction=? AND asset=? AND ((? AND kind='routing_income') OR (amount=? AND (fee=? OR ?)))) ORDER BY (fingerprint=?) DESC,CASE WHEN ? AND kind='routing_income' THEN 2 WHEN fee=? THEN 1 ELSE 0 END DESC,created_at DESC LIMIT 1").map_err(database_error)?;
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
                string(&r.fields, "external_id"),
                r.fields["direction"],
                r.fields["asset"],
                routing,
                r.fields["amount"],
                r.fields["fee"],
                relax,
                fp,
                routing,
                r.fields["fee"],
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
/// `_same_lnd_settlement_identity`: a date refresh cannot repair or
/// reinterpret conflicting native identity.
fn same_lnd_settlement(existing: &Fields, r: &Record) -> bool {
    let incoming = &r.fields;
    let same =
        |key: &str, value: &str| string(existing, key) == value && string(incoming, key) == value;
    if !(same("kind", "lnd_pay")
        && existing.get("external_id") == incoming.get("external_id")
        && same("direction", "outbound")
        && same("asset", "BTC")
        && truthy(existing.get("payment_hash"))
        && existing.get("payment_hash") == incoming.get("payment_hash")
        && same("payment_hash_source", "lnd")
        && integer_or_zero(existing.get("amount")) == integer_or_zero(incoming.get("amount"))
        && integer_or_zero(existing.get("fee")) == integer_or_zero(incoming.get("fee")))
    {
        return false;
    }
    let payload = |fields: &Fields| -> Json {
        serde_json::from_str::<Json>(&string(fields, "raw_json"))
            .ok()
            .filter(Json::is_object)
            .unwrap_or_else(|| json!({}))
    };
    let (old, new) = (payload(existing), payload(incoming));
    let present =
        |v: &Json| !v.is_null() && v != &json!("") && v != &json!(false) && v != &json!(0);
    if !(old["chain"] == json!("lightning")
        && new["chain"] == json!("lightning")
        && present(&old["network"])
        && present(&new["network"])
        && old["chain_instance_id"] == new["chain_instance_id"])
    {
        return false;
    }
    let network = |v: &Json| v.as_str().map_or_else(|| v.to_string(), str::to_owned);
    match (
        crate::descriptors::normalize_network("bitcoin", &network(&old["network"])),
        crate::descriptors::normalize_network("bitcoin", &network(&new["network"])),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn integer_or_zero(value: Option<&Sql>) -> i64 {
    match value {
        Some(Sql::Integer(n)) => *n,
        Some(Sql::Real(n)) => *n as i64,
        Some(Sql::Text(t)) => t.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn merge(existing: &Fields, r: &Record, fp: &str, settlement: bool) -> Result<Fields, AppError> {
    let mut updates = Fields::new();
    let incoming = &r.fields;
    let raw: Json = serde_json::from_str(&string(existing, "raw_json")).unwrap_or(Json::Null);
    let raw_incoming: Json =
        serde_json::from_str(&string(incoming, "raw_json")).unwrap_or(Json::Null);
    let same_source = raw.get("source").is_some_and(|source| {
        kassiber_pytext::loads(&config::stored_json(source))
            .is_ok_and(|v| kassiber_envelope::truthy(&v))
            && Some(source) == raw_incoming.get("source")
    });
    if string(incoming, "kind") == "routing_income"
        && (existing.get("amount") != incoming.get("amount")
            || existing.get("fee") != incoming.get("fee"))
    {
        for key in ["amount", "fee"] {
            updates.insert(key.into(), incoming[key].clone());
        }
        if string(existing, "fingerprint") != fp {
            updates.insert("fingerprint".into(), Sql::Text(fp.into()));
        }
        let rate = existing
            .get("fiat_rate_exact")
            .filter(|v| truthy(Some(v)))
            .or_else(|| existing.get("fiat_rate"));
        let rate = match rate {
            Some(Sql::Text(s)) => Some(s.clone()),
            Some(Sql::Real(n)) => Some(kassiber_pyjson::float_repr(*n)),
            Some(Sql::Integer(n)) => Some(n.to_string()),
            _ => None,
        };
        if let Some(rate) = rate {
            let value = PriceNumber::import_price(&rate, "fiat_rate")?
                .multiply(&r.amount)?
                .finite("fiat_value")?;
            updates.insert("fiat_value_exact".into(), Sql::Text(value.exact()?));
            updates.insert("fiat_value".into(), value.float()?);
        } else if ["fiat_value", "fiat_value_exact"]
            .iter()
            .any(|key| existing.get(*key).is_some_and(|v| *v != Sql::Null))
        {
            updates.insert("fiat_value_exact".into(), Sql::Null);
            updates.insert("fiat_value".into(), Sql::Null);
        }
    }
    let import_authored = |key| {
        !truthy(existing.get(key))
            || kassiber_pytext::loads(&string(existing, "raw_json"))
                .ok()
                .and_then(|raw| kassiber_envelope::get(&raw, key).cloned())
                .filter(|v| v != &Py::Null)
                .is_some_and(|v| kassiber_envelope::py_str(&v) == string(existing, key))
    };
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
    // `authoritative_confirmation_changed` for a live LND settlement.
    let settled = settlement
        && incoming
            .get("confirmed_at")
            .is_some_and(|v| *v != Sql::Null)
        && (existing.get("confirmed_at") != incoming.get("confirmed_at")
            || existing.get("occurred_at") != incoming.get("occurred_at"));
    if confirmed || settled {
        updates.insert("confirmed_at".into(), incoming["confirmed_at"].clone());
        if settlement {
            if existing.get("occurred_at") != incoming.get("occurred_at") {
                updates.insert("occurred_at".into(), incoming["occurred_at"].clone());
            }
            if string(existing, "fingerprint") != fp {
                updates.insert("fingerprint".into(), Sql::Text(fp.into()));
            }
        }
    }
    let price = incoming
        .get("pricing_source_kind")
        .is_some_and(|v| *v != Sql::Null);
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
    } else if (confirmed || settled)
        && if settlement && truthy(existing.get("pricing_source_kind")) {
            // Typed provenance wins over a missing or inconsistent legacy label.
            ["fmv_provider", "manual_rate_cache"]
                .contains(&string(existing, "pricing_source_kind").as_str())
        } else {
            string(existing, "fiat_price_source") == "rates_cache"
        }
    {
        for key in PRICE {
            updates.insert((*key).into(), Sql::Null);
        }
    }
    let exchange_overrides = string(incoming, "pricing_source_kind") == "exchange_execution"
        && priority(&string(incoming, "pricing_source_kind"), "")
            >= priority(
                &string(existing, "pricing_source_kind"),
                &string(existing, "fiat_price_source"),
            );
    for key in ["kind", "description", "counterparty"] {
        let refresh_kind = key == "kind"
            && same_source
            && raw.get("source") == Some(&json!("wasabi_gethistory"))
            && ["coinjoin", "deposit", "withdrawal"].contains(&string(existing, key).as_str());
        if (!truthy(existing.get(key))
            || exchange_overrides && import_authored(key)
            || refresh_kind)
            && truthy(incoming.get(key))
            && (key != "kind" || incoming.get(key) != existing.get(key))
        {
            updates.insert(key.into(), incoming[key].clone());
        }
    }
    let nested = raw
        .get("raw_json")
        .map(|v| {
            v.as_str()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| v.clone())
        })
        .unwrap_or(Json::Null);
    let boundary_import_authored = truthy(existing.get("privacy_boundary"))
        && privacy_boundary(&raw, &nested).ok().flatten().as_ref()
            == Some(&string(existing, "privacy_boundary"));
    if truthy(incoming.get("privacy_boundary"))
        && incoming.get("privacy_boundary") != existing.get("privacy_boundary")
        && (!truthy(existing.get("privacy_boundary")) || boundary_import_authored)
    {
        updates.insert(
            "privacy_boundary".into(),
            incoming["privacy_boundary"].clone(),
        );
    } else if truthy(existing.get("privacy_boundary"))
        && !truthy(incoming.get("privacy_boundary"))
        && boundary_import_authored
        && same_source
    {
        updates.insert("privacy_boundary".into(), Sql::Null);
    }
    if !truthy(existing.get("payment_hash")) && truthy(incoming.get("payment_hash")) {
        for key in ["payment_hash", "payment_hash_source"] {
            updates.insert(key.into(), incoming[key].clone());
        }
    } else if truthy(existing.get("payment_hash"))
        && existing.get("payment_hash") == incoming.get("payment_hash")
        && string(existing, "payment_hash_source") == "chain_script"
        && string(incoming, "payment_hash_source") == "chain_script_unique_outpoint"
    {
        updates.insert(
            "payment_hash_source".into(),
            incoming["payment_hash_source"].clone(),
        );
    }
    if !truthy(existing.get("swap_refund_funding_txid"))
        && truthy(incoming.get("swap_refund_funding_txid"))
    {
        for key in ["swap_refund_funding_txid", "swap_refund_funding_vout"] {
            updates.insert(key.into(), incoming[key].clone());
        }
    } else if truthy(incoming.get("swap_refund_funding_txid"))
        && existing.get("swap_refund_funding_txid") == incoming.get("swap_refund_funding_txid")
        && existing.get("swap_refund_funding_vout") == Some(&Sql::Null)
        && incoming.get("swap_refund_funding_vout") != Some(&Sql::Null)
    {
        updates.insert(
            "swap_refund_funding_vout".into(),
            incoming["swap_refund_funding_vout"].clone(),
        );
    }
    // A file cannot replace an observer's committed graph or chain legs.
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
    // `_may_replace_observation_payload`: a native settlement keeps full power
    // over its own evidence.
    if (!updates.is_empty() || graph_upgrade)
        && (settlement || !committed && (graph_upgrade || !legs(&raw) || legs(&raw_incoming)))
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
    execute_with_ledger_map(conn, request, None, identity)
}
pub fn execute_with_ledger_map(
    conn: &Connection,
    request: Request,
    column_map: Option<&ledger::ColumnMap>,
    identity: Option<&Identity>,
) -> Result<Py, AppError> {
    run(conn, request, column_map, None, identity)
}

/// `import_wasabi_bundle_payload_into_wallet`: the desktop's inline Wasabi
/// bundle. Only the reply's source differs from a bundle file.
pub fn execute_wasabi_payload(
    conn: &Connection,
    request: Request,
    payload: Py,
    identity: Option<&Identity>,
) -> Result<Py, AppError> {
    let mut result = run(conn, request, None, Some(payload), identity)?;
    kassiber_envelope::set(&mut result, "source", s("inline:wasabi_bundle"));
    if let Py::Dict(items) = &mut result {
        items.retain(|(key, _)| *key != kassiber_pyjson::Key::from("file"));
    }
    Ok(result)
}

/// What the desktop's `ui.wallets.import_file` and `ui.wallets.import_samourai`
/// ask the book to import before the local journal step.
pub enum DesktopImport {
    File {
        request: Request,
        column_map: Option<ledger::ColumnMap>,
    },
    WasabiPayload {
        request: Request,
        payload: Py,
    },
    Samourai(samourai::Request),
}

fn run(
    conn: &Connection,
    request: Request,
    column_map: Option<&ledger::ColumnMap>,
    inline: Option<Py>,
    identity: Option<&Identity>,
) -> Result<Py, AppError> {
    let mut ids = Ids { identity, at: 0 };
    let mode = if request.format.active() {
        let mode = exchange::mode(
            request
                .mode
                .as_deref()
                .filter(|v| !v.is_empty())
                .or(Some("full")),
        )?;
        if mode != "full" {
            return Err(AppError::new(
                "validation",
                format!(
                    "{} is an active custodial ledger import",
                    request.format.name()
                ),
            )
            .with_hint(
                "Import the complete platform ledger, then review custody-transfer candidates.",
            ));
        }
        Some(mode)
    } else if request.format.evidence() {
        Some(exchange::mode(request.mode.as_deref())?)
    } else {
        None
    };
    let wallet = if mode == Some("full") {
        Some(exchange::wallet(conn, &request, &mut ids)?)
    } else if mode.is_none() || !request.wallet.is_empty() {
        Some(resolve(conn, &request.profile_id, &request.wallet)?)
    } else {
        None
    };
    if request.format.migration() {
        let wallet = wallet.as_ref().expect("migration wallet");
        let kind: String = conn
            .query_row("SELECT kind FROM wallets WHERE id=?", [&wallet.id], |r| {
                r.get(0)
            })
            .map_err(database_error)?;
        if kind != request.format.migration_kind() {
            return Err(AppError::new("migration_wallet_kind_required", format!("{} requires a dedicated {} wallet", request.format.name(), request.format.migration_kind()))
                .with_hint("Create the provider migration wallet first. Do not import provider history into a descriptor or other authoritative wallet.")
                .with_details(dict([("wallet",s(wallet.label.as_str())),("wallet_kind",s(kind.as_str()))])));
        }
    }
    let from_file = inline.is_none();
    let bundle = match inline {
        Some(payload) => Some(wasabi::load_payload(payload)?),
        None if request.format == Format::Wasabi => Some(wasabi::load(&request.file)?),
        None => None,
    };
    let rows = if let Some(bundle) = &bundle {
        wasabi::rows(&bundle.records)?
    } else if request.format == Format::Ledger {
        ledger::load(&request.file, column_map)?
    } else {
        request.format.load(&request.file)?
    };
    // A Bull Bitcoin wallet imports only the rows of its configured network.
    let (rows, bull_wallet) = if request.format == Format::BullWallet {
        let total = rows.len();
        let network = bull_wallet_network(conn, wallet.as_ref().expect("wallet import"))?;
        (
            bull_wallet::filter(rows, network.as_deref()),
            Some((total, network)),
        )
    } else {
        (rows, None)
    };
    // Relevant mode normalizes and validates each row in reference order;
    // a matched price refusal precedes normalization of any later row.
    let records = if mode == Some("relevant") {
        Vec::new()
    } else {
        rows.iter().map(normalize).collect::<Result<Vec<_>, _>>()?
    };
    kassiber_store::begin_write_savepoint(conn, "wallet_file_import").map_err(database_error)?;
    let result = if mode == Some("relevant") {
        exchange::enrich(conn, &request, &rows)
    } else {
        apply(
            conn,
            &request,
            wallet.as_ref().expect("wallet import"),
            &rows,
            &records,
            &mut ids,
            column_map,
        )
        .and_then(|result| {
            if let Some(bundle) = &bundle {
                let wallet = wallet.as_ref().expect("wallet import");
                wasabi::apply(conn, &request, wallet, bundle, &mut ids, result).and_then(|result| {
                    if from_file {
                        wasabi::record_batch(conn, &request, wallet, &mut ids, result)
                    } else {
                        Ok(result)
                    }
                })
            } else {
                Ok(result)
            }
        })
    };
    if result.is_ok() {
        conn.execute_batch("RELEASE wallet_file_import")
            .map_err(database_error)?;
    } else {
        conn.execute_batch("ROLLBACK TO wallet_file_import;RELEASE wallet_file_import")
            .map_err(database_error)?;
    }
    let result = result.map(|mut outcome| {
        if let (Some((total, network)), Py::Dict(pairs)) = (&bull_wallet, &mut outcome) {
            let at = pairs
                .iter()
                .position(|(key, _)| matches!(key, kassiber_pyjson::Key::Str(k) if k.as_str() == Some("input_format")))
                .unwrap_or(pairs.len());
            let mut extra = vec![
                ("bullbitcoin_wallet_rows", Py::from(rows.len() as i64)),
                ("bullbitcoin_wallet_rows_total", Py::from(*total as i64)),
            ];
            if let Some(network) = network {
                extra.push(("bullbitcoin_wallet_network", s(network.as_str())));
            }
            for (offset, (key, value)) in extra.into_iter().enumerate() {
                pairs.insert(at + offset, (kassiber_pyjson::Key::from(key), value));
            }
        }
        outcome
    });
    result.map_err(|mut error| {
        // Python's SQLCipher binding uses zero-based, untyped messages. Salt
        // presence identifies the active codec without reading or exposing keys.
        if let Some(rest) = error.message.strip_prefix("Error binding parameter ") {
            if let Some((index, _)) = rest.split_once(": type '") {
                let encrypted = conn
                    .query_row("PRAGMA cipher_salt", [], |row| row.get::<_, String>(0))
                    .optional()
                    .ok()
                    .flatten()
                    .is_some();
                if encrypted {
                    if let Ok(index) = index.parse::<usize>() {
                        error.message = format!(
                            "Error binding parameter {} - probably unsupported type.",
                            index - 1
                        );
                    }
                }
            }
        }
        error
    })
}
/// `wallet_bullbitcoin_wallet_network`: the wallet's configured network,
/// where its configuration is a dict that names one.
fn bull_wallet_network(
    conn: &Connection,
    wallet: &crate::Wallet,
) -> Result<Option<String>, AppError> {
    let configuration: Option<String> = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id=?",
            [&wallet.id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let configuration = configuration.filter(|c| !c.is_empty());
    let Ok(Py::Dict(pairs)) = kassiber_pytext::loads(configuration.as_deref().unwrap_or("{}"))
    else {
        return Ok(None);
    };
    let value = pairs
        .into_iter()
        .rev()
        .find(|(key, _)| matches!(key, kassiber_pyjson::Key::Str(k) if k.as_str() == Some("bullbitcoin_wallet_network")))
        .map(|(_, value)| value);
    match value {
        None | Some(Py::Null) => Ok(None),
        Some(Py::Str(text)) if text.as_str() == Some("") => Ok(None),
        Some(value) => crate::bull_connect::network(&value).map(Some),
    }
}
/// `_emit_import_progress`, for a caller collecting progress.
fn import_progress(wallet: &str, processed: usize, total: usize, counts: Option<(usize, i64)>) {
    if !crate::progress::active() {
        return;
    }
    let mut event = kassiber_envelope::dict([
        ("phase", Py::Str("importing".into())),
        ("wallet", Py::Str(wallet.into())),
        ("processed", Py::Int(processed.into())),
        ("total", Py::Int(total.into())),
    ]);
    if let Some((imported, skipped)) = counts {
        kassiber_envelope::set(&mut event, "imported", Py::Int(imported.into()));
        kassiber_envelope::set(&mut event, "skipped", Py::Int(skipped.into()));
    }
    crate::progress::emit(event);
}

/// The rows one import inserted or enriched (`insert_wallet_records`).
struct Inserted {
    imported: Vec<Py>,
    updated: Vec<Py>,
    unchanged: i64,
    skipped: i64,
    inserted_ids: Vec<String>,
    /// `reset_decisions` while the book keeps "Reset book data" records.
    reset: Option<Py>,
}

/// `insert_wallet_records`' row loop: enrich a matching transaction or insert
/// a new one. `match_existing_only` never inserts; unmatched rows are skipped.
fn insert_records(
    conn: &Connection,
    scope: (&String, &String, &String),
    wallet: &crate::Wallet,
    records: &[Record],
    ids: &mut Ids<'_>,
    match_existing_only: bool,
) -> Result<Inserted, AppError> {
    insert_settled_records(conn, scope, wallet, records, ids, match_existing_only, None)
}

/// [`insert_records`] for an LND wallet's import, whose live settlement
/// evidence may correct a stored payment's date.
fn insert_settled_records(
    conn: &Connection,
    scope: (&String, &String, &String),
    wallet: &crate::Wallet,
    records: &[Record],
    ids: &mut Ids<'_>,
    match_existing_only: bool,
    settlement_ids: Option<&[String]>,
) -> Result<Inserted, AppError> {
    let (profile, workspace, currency) = scope;
    let mut imported = Vec::new();
    let mut updated = Vec::new();
    let mut unchanged = 0;
    let mut skipped = 0;
    let mut inserted_ids = Vec::new();
    let reset_active = kassiber_setup::book_reset::has_state(conn, profile)?;
    let mut reset_rebound = 0_i64;
    // Stored ID-less rows match by economic identity and occurrence number,
    // including old positional IDs; matching rewrites no stored row.
    let identities: Vec<_> = records
        .iter()
        .map(|r| {
            synthetic_csv_identity(
                &string(&r.fields, "external_id"),
                &string(&r.fields, "raw_json"),
            )
        })
        .collect();
    let stored: BTreeMap<String, BTreeMap<i64, crate::import_identity::Member>> =
        if identities.iter().any(Option::is_some) {
            crate::import_identity::stored_synthetic_occurrences(conn, profile, Some(&wallet.id))?
                .into_iter()
                .map(|((_, economic), members)| {
                    (economic, crate::import_identity::occurrence_slots(&members))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
    let mut incoming: BTreeMap<String, i64> = BTreeMap::new();
    let total = records.len();
    import_progress(&wallet.label, 0, total, None);
    for (index, (r, identity)) in records.iter().zip(&identities).enumerate() {
        let index = index + 1;
        if let Some((economic, _, _)) = identity {
            let occurrence = incoming.entry(economic.clone()).or_default();
            *occurrence += 1;
            if stored
                .get(economic)
                .and_then(|slots| slots.get(occurrence))
                .is_some_and(|row| row.external_id != string(&r.fields, "external_id"))
            {
                skipped += 1;
                unchanged += 1;
                if index % 200 == 0 || index == total {
                    import_progress(&wallet.label, index, total, Some((imported.len(), skipped)));
                }
                continue;
            }
        }
        // Python's fingerprint converts the amount to msat, after the
        // network guard and the occurrence match, before this row's lookups.
        if let Some(error) = &r.deferred {
            return Err(error.clone());
        }
        let fp = fingerprint(&wallet.id, r);
        for key in ["external_id", "asset"] {
            if let Some(error) = r.binding_errors.get(key) {
                return Err(error.clone());
            }
        }
        let mut existing = find(conn, &wallet.id, r, &fp)?;
        if existing.is_none()
            && r.fingerprint_identity.is_some()
            && string(&r.fields, "external_id_kind") == "txid"
        {
            let mut idless = Record {
                fields: r.fields.clone(),
                amount: r.amount.clone(),
                fee: r.fee.clone(),
                fingerprint_identity: r.fingerprint_identity.clone(),
                ignore_fee: false,
                binding_errors: BTreeMap::new(),
                economics: r.economics,
                tolerance: r.tolerance,
                deferred: None,
            };
            idless
                .fields
                .insert("external_id".into(), Sql::Text(String::new()));
            existing = find(conn, &wallet.id, &idless, &fingerprint(&wallet.id, &idless))?
                .filter(|row| row.get("external_id").is_none_or(|v| *v == Sql::Null));
        }
        let fiat = string(&r.fields, "fiat_currency");
        price_currency(r, currency)?;
        if let Some(existing) = existing {
            let id = string(&existing, "id");
            let settlement = settlement_ids.is_some_and(|settled| {
                settled.contains(&string(&r.fields, "external_id"))
                    && string(&existing, "wallet_id") == wallet.id
                    && same_lnd_settlement(&existing, r)
            });
            let changes = merge(&existing, r, &fp, settlement)?;
            if changes.is_empty() {
                unchanged += 1;
            } else {
                for key in ["kind", "description", "counterparty"]
                    .into_iter()
                    .chain(changes.keys().map(String::as_str))
                {
                    if changes.contains_key(key) {
                        if let Some(error) = r.binding_errors.get(key) {
                            let mut error = error.clone();
                            if let Some((_, suffix)) = error.message.split_once(": type '") {
                                let prefix = changes
                                    .keys()
                                    .filter(|key| {
                                        ![
                                            "kind",
                                            "privacy_boundary",
                                            "description",
                                            "counterparty",
                                            "payment_hash",
                                            "payment_hash_source",
                                            "swap_refund_funding_txid",
                                            "swap_refund_funding_vout",
                                            "raw_json",
                                        ]
                                        .contains(&key.as_str())
                                    })
                                    .count();
                                let metadata =
                                    ["kind", "privacy_boundary", "description", "counterparty"];
                                let index = prefix
                                    + metadata
                                        .iter()
                                        .take_while(|field| **field != key)
                                        .filter(|field| changes.contains_key(**field))
                                        .count()
                                    + 1;
                                error.message =
                                    format!("Error binding parameter {index}: type '{suffix}");
                            }
                            return Err(error);
                        }
                    }
                }
                update(conn, "transactions", &id, &changes)?;
                updated.push(change(
                    &id,
                    &wallet.label,
                    r,
                    changes.keys().cloned().collect(),
                ));
            }
            skipped += 1;
        } else if match_existing_only {
            skipped += 1;
        } else {
            // After "Reset book data" the re-observed row takes its old id
            // back, so every decision keyed by it binds again.
            let reclaimed = if reset_active {
                let external = string(&r.fields, "external_id");
                kassiber_setup::book_reset::claim_transaction_id(
                    conn,
                    profile,
                    &wallet.id,
                    &fp,
                    (!external.is_empty()).then_some(external.as_str()),
                    &string(&r.fields, "asset"),
                    &string(&r.fields, "direction"),
                )?
            } else {
                None
            };
            let id = match &reclaimed {
                Some(id) => id.clone(),
                None => ids.next(conn)?,
            };
            if let Some(error) = r.insert_binding_error() {
                return Err(error.clone());
            }
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
            if reclaimed.is_some()
                && kassiber_setup::book_reset::settle_claim(conn, &id)?
                    == kassiber_setup::book_reset::Settled::BoundWithMetadata
            {
                reset_rebound += 1;
            }
            imported.push(change(
                &id,
                &wallet.label,
                r,
                vec!["metadata".into(), "pricing".into(), "transaction".into()],
            ));
            inserted_ids.push(id);
        }
        if index % 200 == 0 || index == total {
            import_progress(&wallet.label, index, total, Some((imported.len(), skipped)));
        }
    }
    let reset = if reset_active {
        reset_rebound += kassiber_setup::book_reset::rebind_parked(conn, profile)?;
        let mut counts = kassiber_setup::book_reset::counts(conn, profile)?;
        if let Py::Dict(items) = &mut counts {
            items.insert(0, ("rebound".into(), Py::from(reset_rebound)));
        }
        Some(counts)
    } else {
        None
    };
    Ok(Inserted {
        imported,
        updated,
        unchanged,
        skipped,
        inserted_ids,
        reset,
    })
}
fn apply(
    conn: &Connection,
    request: &Request,
    wallet: &crate::Wallet,
    rows: &[csv::Row],
    records: &[Record],
    ids: &mut Ids<'_>,
    column_map: Option<&ledger::ColumnMap>,
) -> Result<Py, AppError> {
    apply_records(conn, request, wallet, rows, records, ids, column_map, None)
}
#[allow(clippy::too_many_arguments)]
fn apply_records(
    conn: &Connection,
    request: &Request,
    wallet: &crate::Wallet,
    rows: &[csv::Row],
    records: &[Record],
    ids: &mut Ids<'_>,
    column_map: Option<&ledger::ColumnMap>,
    api_source: Option<&str>,
) -> Result<Py, AppError> {
    let profile = &request.profile_id;
    let (workspace, currency): (String, String) = conn
        .query_row(
            "SELECT workspace_id,fiat_currency FROM profiles WHERE id=?",
            [profile],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(database_error)?;
    let (kind, configuration): (String, String) = conn
        .query_row(
            "SELECT kind,config_json FROM wallets WHERE id=?",
            [&wallet.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(database_error)?;
    let configuration = serde_json::from_str(&configuration).unwrap_or(Json::Null);
    for r in records {
        let raw = serde_json::from_str(&string(&r.fields, "raw_json")).unwrap_or(Json::Null);
        network::guard_import(
            conn,
            profile,
            &kind,
            &configuration,
            &raw,
            &string(&r.fields, "asset"),
        )?;
    }
    let reconciliation = if request.format.evidence() {
        Some(
            records
                .iter()
                .map(|r| exchange::find_profile(conn, profile, r, Some(&wallet.id)))
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else {
        None
    };
    let Inserted {
        imported,
        updated,
        unchanged,
        skipped,
        inserted_ids,
        reset,
    } = insert_records(
        conn,
        (profile, &workspace, &currency),
        wallet,
        records,
        ids,
        false,
    )?;
    let invalidated = !imported.is_empty() || !updated.is_empty();
    if invalidated {
        invalidate_journals(conn, profile)?;
    }
    let format = request.format.name();
    let updated_count = updated.len() as i64;
    let mut result = vec![
        ("wallet", s(wallet.label.as_str())),
        (
            "source",
            s(match api_source {
                Some(source) => source.to_owned(),
                None if request.format == Format::Document => format.into(),
                None => format!("file:{format}"),
            }),
        ),
        ("imported", Py::from(imported.len() as i64)),
        ("skipped", Py::from(skipped)),
        ("unchanged", Py::from(unchanged)),
        ("journal_invalidated", Py::Bool(invalidated)),
        ("inserted_records", Py::List(imported)),
        ("updated_records", Py::List(updated)),
    ];
    if let Some(reset) = reset {
        result.push(("reset_decisions", reset));
    }
    if api_source.is_some() {
        if updated_count > 0 {
            result.push(("updated", Py::from(updated_count)));
        }
        return Ok(dict(result));
    }
    if request.format == Format::Phoenix {
        phoenix::metadata(
            conn,
            (profile, &workspace),
            &wallet.id,
            rows,
            ids,
            &mut result,
            invalidated,
        )?;
    }
    if request.format == Format::River {
        river::metadata(
            conn,
            (profile, &workspace),
            &wallet.id,
            rows,
            ids,
            &mut result,
            invalidated,
        )?;
    }
    if let Some(reconciliation) = reconciliation {
        if updated_count > 0 {
            result.push(("updated", Py::from(updated_count)));
        }
        exchange::reconcile(
            conn,
            request,
            wallet,
            records,
            &reconciliation,
            ids,
            &mut result,
        )?;
    }
    if request.format.active() && updated_count > 0 {
        result.push(("updated", Py::from(updated_count)));
    }
    if request.format == Format::TwentyOne {
        exchange::clear_legacy_flags(conn, request, wallet, records, &mut result)?;
        // Reactivation changes journal inputs even without a pricing refresh.
        if !invalidated && result.iter().any(|(key, _)| *key == "reactivated") {
            invalidate_journals(conn, profile)?;
            result
                .iter_mut()
                .find(|(key, _)| *key == "journal_invalidated")
                .unwrap()
                .1 = Py::Bool(true);
        }
    }
    if request.format.migration() {
        tax_platform::reconcile(conn, request, wallet, ids, &mut result, invalidated)?;
    }
    if matches!(request.format, Format::BtcpayCsv | Format::BtcpayJson) {
        btcpay::metadata(
            conn,
            (profile, &workspace),
            &wallet.id,
            rows,
            ids,
            &mut result,
            invalidated,
        )?;
    }
    if !inserted_ids.is_empty() && !matches!(request.format, Format::Wasabi | Format::Document) {
        let batch = ids.next(conn)?;
        conn.execute("INSERT INTO import_batches(id,workspace_id,profile_id,wallet_id,source_format,source_filename,column_map_json,imported_at,rows_inserted,rows_updated,rows_skipped) VALUES(?,?,?,?,?,?,?,?,?,?,?)",params![batch,workspace,profile,wallet.id,format,std::path::Path::new(&request.file).file_name().unwrap_or_default().to_string_lossy(),column_map.map(|map| config::stored_json(&map.raw)),ids.time(conn)?,inserted_ids.len() as i64,if request.format.evidence() || request.format.active() { updated_count } else { 0 },skipped]).map_err(database_error)?;
        // Each row's place among the run's created rows is its source
        // position among same-time rows (`kassiber_store::transaction_order`).
        for (position, id) in (1_i64..).zip(inserted_ids) {
            conn.execute("INSERT OR IGNORE INTO import_batch_transactions(batch_id,transaction_id,source_position) VALUES(?,?,?)",params![batch,id,position]).map_err(database_error)?;
        }
        result.push(("import_batch_id", s(batch)));
    }
    if request.format.evidence() || request.format.active() {
        result.push((request.format.provider().2, Py::from(records.len() as i64)));
    }
    if request.format == Format::Document {
        if updated_count > 0 {
            result.push(("updated", Py::from(updated_count)));
        }
        return Ok(dict(result));
    }
    result.push(("input_format", s(format)));
    let path = kassiber_paths::abspath(std::path::Path::new(&request.file))
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    result.push(("file", s(path.to_string_lossy().into_owned())));
    if request.format.active() {
        result.push(("mode", s("full")));
    }
    Ok(dict(result))
}
#[cfg(test)]
mod tests;

/// A sync route's rows for `import_records_into_wallet`, not a file import:
/// no import batch, no input format, and the source is the route's label.
pub(crate) struct RecordImport<'a> {
    pub profile_id: &'a str,
    pub wallet_id: &'a str,
    pub label: &'a str,
    pub match_existing_only: bool,
    pub report_updates: bool,
    pub btcpay: bool,
    /// `authoritative_settlement_ids`: LND payments whose live settlement
    /// timing may correct a previous import's date.
    pub settlement_ids: &'a [String],
}

/// `import_records_into_wallet(..., commit=False)`; the caller owns the
/// transaction.
pub(crate) fn import_records(
    conn: &Connection,
    spec: &RecordImport<'_>,
    rows: &[csv::Row],
) -> Result<Py, AppError> {
    let profile = spec.profile_id.to_owned();
    let (workspace, currency): (String, String) = conn
        .query_row(
            "SELECT workspace_id,fiat_currency FROM profiles WHERE id=?",
            [&profile],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(database_error)?;
    let scoped: Option<(String, String, String)> = conn
        .query_row(
            "SELECT label,kind,config_json FROM wallets WHERE id=? AND profile_id=?",
            [spec.wallet_id, &profile],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(database_error)?;
    let Some((label, kind, configuration)) = scoped else {
        return Err(AppError::new(
            "not_found",
            "Import wallet does not belong to the selected book",
        ));
    };
    let wallet = crate::Wallet {
        id: spec.wallet_id.to_owned(),
        label,
        account_code: None,
    };
    let records = rows
        .iter()
        .map(|row| normalize_labeled(row, spec.label))
        .collect::<Result<Vec<_>, _>>()?;
    let configuration = serde_json::from_str(&configuration).unwrap_or(Json::Null);
    for r in &records {
        let raw = serde_json::from_str(&string(&r.fields, "raw_json")).unwrap_or(Json::Null);
        network::guard_import(
            conn,
            &profile,
            &kind,
            &configuration,
            &raw,
            &string(&r.fields, "asset"),
        )?;
    }
    let mut ids = Ids {
        identity: None,
        at: 0,
    };
    let inserted = insert_settled_records(
        conn,
        (&profile, &workspace, &currency),
        &wallet,
        &records,
        &mut ids,
        spec.match_existing_only,
        (kind == "lnd").then_some(spec.settlement_ids),
    )?;
    let invalidated = !inserted.imported.is_empty() || !inserted.updated.is_empty();
    if invalidated {
        invalidate_journals(conn, &profile)?;
    }
    let updated_count = inserted.updated.len() as i64;
    let mut result = vec![
        ("wallet", s(wallet.label.as_str())),
        ("source", s(spec.label)),
        ("imported", Py::from(inserted.imported.len() as i64)),
        ("skipped", Py::from(inserted.skipped)),
        ("unchanged", Py::from(inserted.unchanged)),
        ("journal_invalidated", Py::Bool(invalidated)),
        ("inserted_records", Py::List(inserted.imported)),
        ("updated_records", Py::List(inserted.updated)),
    ];
    if let Some(reset) = inserted.reset {
        result.push(("reset_decisions", reset));
    }
    if spec.report_updates && updated_count > 0 {
        result.push(("updated", Py::from(updated_count)));
    }
    if spec.btcpay {
        btcpay::metadata(
            conn,
            (&profile, &workspace),
            &wallet.id,
            rows,
            &mut ids,
            &mut result,
            invalidated,
        )?;
    }
    Ok(dict(result))
}

/// Bull Bitcoin wallet-history rows of `file`, before any network filter.
pub(crate) fn bull_wallet_rows(file: &str) -> Result<Vec<csv::Row>, AppError> {
    Format::BullWallet.load(file)
}

/// `filter_bullbitcoin_wallet_records` for one normalized network.
pub(crate) fn bull_wallet_filter(rows: Vec<csv::Row>, network: &str) -> Vec<csv::Row> {
    bull_wallet::filter(rows, Some(network))
}

/// `normalize_btcpay_record` over CSV-shaped Greenfield wallet rows.
pub(crate) fn btcpay_rows(rows: Vec<Py>) -> Result<Vec<csv::Row>, AppError> {
    rows.into_iter().map(btcpay::record).collect()
}

/// Plain record dicts (Python `ImportRow`s) a sync route built itself.
pub(crate) fn record_rows(rows: Vec<Py>) -> Vec<csv::Row> {
    rows.into_iter()
        .map(|row| csv::Row {
            data: generic_json::projection(&row),
            headers: vec![],
            rest: None,
            original: Some(row),
        })
        .collect()
}

/// A field of a record dict row.
pub(crate) fn row_field<'a>(row: &'a csv::Row, key: &str) -> &'a Py {
    static NULL: Py = Py::Null;
    row.original
        .as_ref()
        .and_then(|original| kassiber_envelope::get(original, key))
        .unwrap_or(&NULL)
}

/// `record["fee"] = msat_to_btc(...)`, keeping the projection in step.
pub(crate) fn set_row_fee(row: &mut csv::Row, fee: &str) {
    if let Some(original) = &mut row.original {
        kassiber_envelope::set(original, "fee", s(fee));
        row.data = generic_json::projection(original);
    }
}

/// `btc_to_msat`: half-up integral msat of a decimal BTC amount.
pub(crate) fn btc_to_msat(amount: &str) -> Result<i64, AppError> {
    msat(&dec(amount)?)
}

/// `str(msat_to_btc(value))`.
pub(crate) fn msat_to_btc(value: i64) -> String {
    dec(&value.to_string())
        .expect("integer decimal")
        .div(&dec("100000000000").expect("msat scale"), &CTX)
        .expect("msat division")
        .to_string()
}

/// `make_transaction_fingerprint` over already-rendered values.
pub(crate) fn fingerprint_parts(
    wallet: &str,
    external_id: &str,
    occurred_at: &str,
    direction: &str,
    asset: &str,
    amount: &str,
    fee: &str,
) -> String {
    let value = json!({"wallet_id":wallet,"external_id":external_id,"occurred_at":occurred_at,"direction":direction,"asset":asset,"amount":amount,"fee":fee});
    Sha256::digest(config::stored_json(&value).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `apply_btcpay_metadata` alone: notes and tags on matching rows, without
/// the import's journal invalidation.
pub(crate) fn btcpay_metadata(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    rows: &[csv::Row],
) -> Result<Vec<(&'static str, Py)>, AppError> {
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let mut ids = Ids {
        identity: None,
        at: 0,
    };
    let mut result = Vec::new();
    btcpay::metadata(
        conn,
        (profile_id, &workspace),
        wallet_id,
        rows,
        &mut ids,
        &mut result,
        true,
    )?;
    Ok(result)
}
