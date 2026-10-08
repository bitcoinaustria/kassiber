//! The ledger request, schema version 2 (design §3.1-§3.5).
//!
//! Python's builder (`kassiber/core/engines/ledger_request.py`) turns the
//! finalized custody projection into this document: the policy, the
//! configuration names RP2's membership checks accept, the holder, the
//! wallets, one typed row per projection row, and the move and cross-asset
//! pairs, each in projection order. It carries facts, never marker text: the
//! engine derives every marker value itself (§3.6).
//!
//! Every type rejects unknown fields and missing ones (a nullable field is
//! written as `null`, never left out; only a failed timestamp's
//! `error_class` may be absent), and serializing a request gives back
//! the document it was read from (`tests/ledger_fixtures.rs` checks both on
//! every stored request), so a builder that drifts fails loudly instead of
//! being half-understood.

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::decimal::Decimal;
use crate::error::{EngineError, EngineResult, ErrorClass};
use crate::model::{ConfigSpec, MethodName, TsInput};

/// The only schema version the ledger operations read and write.
pub const SCHEMA_VERSION: u32 = 2;

/// What a schema-2 request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Build the journal ledger of one finalized projection.
    Ledger,
    /// Stop after the final prepare pass and return the intermediate stages
    /// (design §3.10); test-facing.
    Prepare,
    /// The engine's names and tables (`crate::manifest`).
    Manifest,
}

/// The first fields of every schema-2 document, read before its body so a
/// document of another version fails on its version, not on its shape.
#[derive(Deserialize)]
struct Version {
    schema_version: u32,
}

#[derive(Deserialize)]
struct Head {
    operation: Operation,
}

/// A request's operation, after its schema version was checked.
pub fn operation(request: &str) -> EngineResult<Operation> {
    let version: Version = parse(request)?;
    if version.schema_version != SCHEMA_VERSION {
        return Err(EngineError::request(format!(
            "unsupported schema_version {}; the ledger operations read {SCHEMA_VERSION}",
            version.schema_version
        )));
    }
    let head: Head = parse(request)?;
    Ok(head.operation)
}

/// Parses a request document, or fails with `RequestError`.
pub fn parse<'a, T: Deserialize<'a>>(request: &'a str) -> EngineResult<T> {
    serde_json::from_str(request)
        .map_err(|error| EngineError::request(format!("invalid engine request: {error}")))
}

/// Reads a nullable field the request must still carry. Serde reads a
/// missing `Option` field as `None`; the explicit function makes a missing
/// field an error instead, so a builder that stops sending, say, `hold` is
/// refused rather than read as "no hold".
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// `{"schema_version": 2, "operation": "manifest"}`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRequest {
    pub schema_version: u32,
    pub operation: Operation,
}

/// A `ledger` or `prepare` request: one finalized custody projection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerRequest {
    pub schema_version: u32,
    pub operation: Operation,
    pub policy: Policy,
    /// The names RP2's membership checks accept, after the configuration
    /// checks: sorted, stripped, and as the INI parser reads them.
    pub config: Config,
    /// The profile label, unstripped, as entries carry it.
    #[serde(deserialize_with = "nullable")]
    pub holder: Option<String>,
    /// `wallet_refs_by_id`, in order.
    pub wallets: Vec<Wallet>,
    /// `projection.rows`, in order.
    pub rows: Vec<Row>,
    /// `projection.intra_pairs`, in order.
    pub intra_pairs: Vec<IntraPair>,
    /// `projection.cross_asset_pairs`, in order.
    pub cross_asset_pairs: Vec<CrossAssetPair>,
}

/// The tax policy of the profile (`build_tax_policy`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub country: Country,
    /// The profile's method when the policy allows it, else `null` (the
    /// compute stage then fails with the method gate, design §3.9).
    #[serde(deserialize_with = "nullable")]
    pub method: Option<Method>,
    /// `str(profile["gains_algorithm"])`, for the method gate's message.
    pub method_label: String,
    /// The profile opted into reviewing coarse pricing.
    pub require_coarse_review: bool,
}

/// The country rules.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Country {
    /// Kassiber's generic country: a holding period in whole days, capped at
    /// `sys.maxsize`.
    Generic { long_term_days: u64 },
    /// Austria.
    At {},
}

/// An accounting method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Fifo,
    Lifo,
    Hifo,
    Lofo,
    MovingAverage,
    MovingAverageAt,
}

impl Method {
    /// The lot engine's method.
    pub fn method_name(self) -> MethodName {
        match self {
            Method::Fifo => MethodName::Fifo,
            Method::Lifo => MethodName::Lifo,
            Method::Hifo => MethodName::Hifo,
            Method::Lofo => MethodName::Lofo,
            Method::MovingAverage => MethodName::MovingAverage,
            Method::MovingAverageAt => MethodName::MovingAverageAt,
        }
    }
}

/// The configuration's names.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub assets: Vec<String>,
    pub exchanges: Vec<String>,
    pub holders: Vec<String>,
}

impl Config {
    /// The names as the lot engine's constructors check them.
    pub fn spec(&self) -> ConfigSpec {
        ConfigSpec {
            assets: self.assets.clone(),
            exchanges: self.exchanges.clone(),
            holders: self.holders.clone(),
        }
    }
}

/// One wallet reference. Account fields stay in Python.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wallet {
    pub id: String,
    #[serde(deserialize_with = "nullable")]
    pub label: Option<String>,
}

/// The direction of a projection row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
}

/// An Austrian regime the user chose for a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Regime {
    Alt,
    Neu,
}

impl Regime {
    /// The value as a marker writes it (`alt`, `neu`).
    pub fn as_str(self) -> &'static str {
        match self {
            Regime::Alt => "alt",
            Regime::Neu => "neu",
        }
    }
}

/// Why a row is held out of the ledger. A conflict wins over pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hold {
    /// Another transaction spends the same input and won.
    ConflictingSpend,
    /// The chain reports the transaction unconfirmed.
    PendingOnchainConfirmation,
}

/// One finalized projection row (design §3.3).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// The synthetic projection row id.
    pub id: String,
    /// `journal_transaction_id`: the transaction journals and quarantines
    /// name.
    pub anchor: String,
    pub asset: String,
    pub wallet_id: String,
    pub direction: Direction,
    /// `occurred_at` as Python parsed it; `raw` orders the string
    /// comparisons.
    pub ts: Ts,
    pub amount_msat: u64,
    pub fee_msat: u64,
    /// `fiat_rate_exact`, else `fiat_rate`.
    #[serde(deserialize_with = "nullable")]
    pub rate: Option<DecimalText>,
    /// `fiat_value_exact`, else `fiat_value`.
    #[serde(deserialize_with = "nullable")]
    pub value: Option<DecimalText>,
    /// Priced from a coarse fallback (`pricing_quality == coarse_fallback`).
    pub coarse: bool,
    /// `normalized_transaction_kind(row)`.
    pub kind: String,
    /// The first non-empty of the row's note, description, and kind. The
    /// engine appends it to the notes it composes, where its marker tokens
    /// keep today's effect (§3.6).
    #[serde(deserialize_with = "nullable")]
    pub text: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub regime_override: Option<Regime>,
    #[serde(deserialize_with = "nullable")]
    pub category_override: Option<String>,
    /// `taxability_override == 0`.
    pub taxability_off: bool,
    /// The row carries privacy-hop evidence.
    pub privacy_hop: bool,
    #[serde(deserialize_with = "nullable")]
    pub hold: Option<Hold>,
    #[serde(deserialize_with = "nullable")]
    pub chain_order: Option<ChainOrder>,
}

/// A row's timestamp: what `timestamp_fields` returned for `occurred_at`.
///
/// A parsed timestamp carries every field; one that failed to parse carries
/// the error the constructor raises. The two shapes serialize back as Python
/// wrote them, so a `vienna_date` of `null` stays present.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Ts {
    Parsed(ParsedTs),
    Failed(FailedTs),
}

/// A timestamp Python parsed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParsedTs {
    pub raw: String,
    pub us: i64,
    pub offset_s: i32,
    pub date: String,
    pub year: i32,
    /// `null` when the instant is outside the zone database's range.
    #[serde(deserialize_with = "nullable")]
    pub vienna_date: Option<String>,
    pub display: String,
    pub iso: String,
}

/// A timestamp that failed to parse.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailedTs {
    pub raw: String,
    pub error: String,
    /// `ValueError` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<ErrorClass>,
}

impl Ts {
    /// The lot engine's timestamp input.
    pub fn input(&self) -> TsInput {
        match self {
            Ts::Parsed(ts) => TsInput {
                raw: ts.raw.clone(),
                error: None,
                error_class: None,
                us: Some(ts.us),
                offset_s: Some(ts.offset_s),
                date: Some(ts.date.clone()),
                year: Some(ts.year),
                vienna_date: ts.vienna_date.clone(),
                display: Some(ts.display.clone()),
                iso: Some(ts.iso.clone()),
            },
            Ts::Failed(ts) => TsInput {
                raw: ts.raw.clone(),
                error: Some(ts.error.clone()),
                error_class: ts.error_class,
                us: None,
                offset_s: None,
                date: None,
                year: None,
                vienna_date: None,
                display: None,
                iso: None,
            },
        }
    }

    /// `str(occurred_at)`: the key of every string comparison.
    pub fn raw(&self) -> &str {
        match self {
            Ts::Parsed(ts) => &ts.raw,
            Ts::Failed(ts) => &ts.raw,
        }
    }
}

/// A stored decimal as Python parsed it: the canonical form when finite,
/// otherwise CPython's text for it (`NaN`, `-sNaN`, `Infinity`, ...). A
/// projection row may store non-finite text, and what it means is decided
/// where the engine reads the value, as Python's comparisons decide it.
#[derive(Clone, Debug)]
pub enum DecimalText {
    Finite(Decimal),
    NonFinite(String),
}

/// Whether `text` is CPython's `str()` of a non-finite `Decimal`.
fn is_non_finite_text(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    if unsigned == "Infinity" {
        return true;
    }
    let payload = match unsigned.strip_prefix("sNaN") {
        Some(rest) => rest,
        None => match unsigned.strip_prefix("NaN") {
            Some(rest) => rest,
            None => return false,
        },
    };
    payload.is_empty() || (payload.bytes().all(|b| b.is_ascii_digit()) && !payload.starts_with('0'))
}

impl Serialize for DecimalText {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            DecimalText::Finite(value) => serializer.serialize_str(&value.to_canonical()),
            DecimalText::NonFinite(text) => serializer.serialize_str(text),
        }
    }
}

impl<'de> Deserialize<'de> for DecimalText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if let Ok(value) = Decimal::from_canonical(&text) {
            return Ok(DecimalText::Finite(value));
        }
        if is_non_finite_text(&text) {
            return Ok(DecimalText::NonFinite(text));
        }
        Err(de::Error::custom(format!(
            "not a canonical or non-finite decimal: {text:?}"
        )))
    }
}

/// Chain-order evidence: the move's on-chain scope and the scopes it spends.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainOrder {
    /// `(chain, network, txid, asset)`.
    pub scope: [String; 4],
    /// Sorted.
    pub parents: Vec<[String; 4]>,
}

/// A selected same-asset move (design §3.4).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntraPair {
    /// The outbound row's id.
    pub out: String,
    /// The inbound row's id.
    #[serde(rename = "in")]
    pub inbound: String,
    /// How the pair was proven (`pair.get("source")`).
    #[serde(deserialize_with = "nullable")]
    pub source: Option<String>,
    /// The atomic bundle; `null` leaves the move ungrouped.
    #[serde(deserialize_with = "nullable")]
    pub group_id: Option<String>,
}

/// A cross-asset pair, as the projection lists it (design §3.4).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossAssetPair {
    /// Raw; `null` and `""` fall back differently by use.
    #[serde(deserialize_with = "nullable")]
    pub pair_id: Option<String>,
    pub out_id: String,
    pub in_id: String,
    #[serde(deserialize_with = "nullable")]
    pub out_transaction_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub in_transaction_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub out_asset: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub in_asset: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub policy: Option<String>,
    /// `is_bitcoin_rail_pair(out_asset, in_asset)`.
    pub bitcoin_rail: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_finite_text_is_cpythons() {
        for text in [
            "NaN",
            "-NaN",
            "sNaN",
            "-sNaN",
            "NaN12",
            "sNaN7",
            "Infinity",
            "-Infinity",
        ] {
            assert!(is_non_finite_text(text), "{text}");
        }
        for text in [
            "", "nan", "Inf", "inf", "+NaN", "NaN012", "NaN1a", "1E0", "-", "--NaN",
        ] {
            assert!(!is_non_finite_text(text), "{text}");
        }
    }

    #[test]
    fn decimal_text_round_trips() {
        for text in ["1E0", "-0E0", "12345E-3", "NaN", "-Infinity", "sNaN3"] {
            let json = serde_json::to_string(text).unwrap();
            let value: DecimalText = serde_json::from_str(&json).unwrap();
            assert_eq!(serde_json::to_string(&value).unwrap(), json);
        }
        for text in ["1.0", "1e0", "01E0", "NaN012", "Infinite"] {
            let json = serde_json::to_string(text).unwrap();
            assert!(
                serde_json::from_str::<DecimalText>(&json).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn operation_checks_the_version_first() {
        let error = operation(r#"{"schema_version": 1, "operation": "compute"}"#).unwrap_err();
        assert_eq!(error.class, ErrorClass::RequestError);
        assert!(error.message.starts_with("unsupported schema_version 1"));
        assert_eq!(
            operation(r#"{"schema_version": 2, "operation": "manifest"}"#).unwrap(),
            Operation::Manifest
        );
        let error = operation(r#"{"schema_version": 2, "operation": "compute"}"#).unwrap_err();
        assert_eq!(error.class, ErrorClass::RequestError);
        assert!(error.message.starts_with("invalid engine request:"));
    }
}
