//! The JSON boundary, schema version 1: request and response documents.
//!
//! Decimals cross the boundary in the canonical form `[-]<digits>E<exp>`
//! ([`Dec`]), which keeps the coefficient and exponent. Requests reject
//! unknown fields so a drifting caller fails loudly instead of being
//! half-understood. `docs/reference/tax-engine.md` documents the format.

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::decimal::Decimal;
use crate::error::ErrorClass;

/// The only schema version this engine reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// A decimal in the canonical boundary form.
#[derive(Clone, Debug)]
pub struct Dec(pub Decimal);

impl Serialize for Dec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_canonical())
    }
}

impl<'de> Deserialize<'de> for Dec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Decimal::from_canonical(&text)
            .map(Dec)
            .map_err(|_| de::Error::custom(format!("not a canonical decimal: {text:?}")))
    }
}

impl From<Decimal> for Dec {
    fn from(value: Decimal) -> Self {
        Dec(value)
    }
}

/// Which RP2 constructor an entry stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// `InTransaction`: an acquisition lot, taxable when earn-typed.
    In,
    /// `OutTransaction`: a disposal.
    Out,
    /// `IntraTransaction`: a move between accounts; its fee can be taxable.
    Intra,
}

impl EntryKind {
    /// RP2's class name, used in messages.
    pub fn class_name(self) -> &'static str {
        match self {
            EntryKind::In => "InTransaction",
            EntryKind::Out => "OutTransaction",
            EntryKind::Intra => "IntraTransaction",
        }
    }
}

/// A timestamp as Python parsed it, or the error parsing raised.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TsInput {
    /// `str()` of the value given to the constructor.
    pub raw: String,
    /// The parse error message, when parsing failed.
    #[serde(default)]
    pub error: Option<String>,
    /// The class of that error (`ValueError` when absent).
    #[serde(default)]
    pub error_class: Option<ErrorClass>,
    /// Microseconds since the Unix epoch, absolute.
    #[serde(default)]
    pub us: Option<i64>,
    /// UTC offset in seconds.
    #[serde(default)]
    pub offset_s: Option<i32>,
    /// `datetime.date().isoformat()` in the timestamp's own offset.
    #[serde(default)]
    pub date: Option<String>,
    /// `datetime.year` in the timestamp's own offset.
    #[serde(default)]
    pub year: Option<i32>,
    /// The calendar date in Europe/Vienna, for Austrian rules.
    #[serde(default)]
    pub vienna_date: Option<String>,
    /// `str(datetime)`.
    #[serde(default)]
    pub display: Option<String>,
    /// `datetime.isoformat()`.
    #[serde(default)]
    pub iso: Option<String>,
}

/// One RP2 constructor call: the arguments exactly as given.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryInput {
    pub kind: EntryKind,
    /// The adapter's row number, RP2's `internal_id`.
    pub row: i64,
    /// `null` stands for `None`.
    #[serde(default)]
    pub unique_id: Option<String>,
    pub asset: String,
    /// Required for `in` and `out`; an intra entry is always `MOVE`.
    #[serde(default)]
    pub transaction_type: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub exchange: Option<String>,
    #[serde(default)]
    pub holder: Option<String>,
    #[serde(default)]
    pub from_exchange: Option<String>,
    #[serde(default)]
    pub from_holder: Option<String>,
    #[serde(default)]
    pub to_exchange: Option<String>,
    #[serde(default)]
    pub to_holder: Option<String>,
    #[serde(default)]
    pub spot_price: Option<Dec>,
    #[serde(default)]
    pub crypto_in: Option<Dec>,
    #[serde(default)]
    pub crypto_fee: Option<Dec>,
    #[serde(default)]
    pub fiat_in_no_fee: Option<Dec>,
    #[serde(default)]
    pub fiat_in_with_fee: Option<Dec>,
    #[serde(default)]
    pub fiat_fee: Option<Dec>,
    #[serde(default)]
    pub crypto_out_no_fee: Option<Dec>,
    #[serde(default)]
    pub crypto_out_with_fee: Option<Dec>,
    #[serde(default)]
    pub fiat_out_no_fee: Option<Dec>,
    #[serde(default)]
    pub crypto_sent: Option<Dec>,
    #[serde(default)]
    pub crypto_received: Option<Dec>,
    pub ts: TsInput,
    /// `str(transaction)`, embedded in RP2's negative-balance and
    /// duplicate-entry messages.
    #[serde(default)]
    pub text: String,
}

/// The configuration's known names, for RP2's membership checks.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSpec {
    pub assets: Vec<String>,
    pub exchanges: Vec<String>,
    pub holders: Vec<String>,
}

/// `check_entry`: validate one constructor call.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckEntryRequest {
    pub schema_version: u32,
    /// `check_entry` when present.
    #[serde(default)]
    pub operation: Option<String>,
    /// Membership checks are skipped without it.
    #[serde(default)]
    pub config: Option<ConfigSpec>,
    pub entry: EntryInput,
}

/// What a compute request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// RP2's `compute_tax` for one asset.
    Compute,
    /// The Austrian multi-asset runner.
    ComputeMulti,
    /// The Austrian swap-pair checks.
    Validate,
}

/// The country policy.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CountrySpec {
    /// Kassiber's generic country: a holding period in whole days.
    Generic { long_term_days: u64 },
    /// Austria: never long-term (`sys.maxsize` days).
    At {},
}

/// The accounting method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodName {
    Fifo,
    Lifo,
    Hifo,
    Lofo,
    MovingAverage,
    MovingAverageAt,
}

impl MethodName {
    /// RP2's plugin name.
    pub fn as_str(self) -> &'static str {
        match self {
            MethodName::Fifo => "fifo",
            MethodName::Lifo => "lifo",
            MethodName::Hifo => "hifo",
            MethodName::Lofo => "lofo",
            MethodName::MovingAverage => "moving_average",
            MethodName::MovingAverageAt => "moving_average_at",
        }
    }
}

/// One asset's entries, in insertion order.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetInput {
    pub asset: String,
    pub entries: Vec<EntryInput>,
}

/// `compute`, `compute_multi`, or `validate`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeRequest {
    pub schema_version: u32,
    pub operation: Operation,
    pub country: CountrySpec,
    pub method: MethodName,
    pub assets: Vec<AssetInput>,
}

/// A failure.
#[derive(Clone, Debug, Serialize)]
pub struct ErrorBody {
    pub class: ErrorClass,
    pub message: String,
}

/// The response to a failed call.
#[derive(Clone, Debug, Serialize)]
pub struct ErrorResponse {
    pub schema_version: u32,
    pub ok: bool,
    pub error: ErrorBody,
}

/// The values RP2's constructor stores and derives. Fields that do not
/// apply to the entry's kind are `null`.
#[derive(Clone, Debug, Serialize)]
pub struct DerivedEntry {
    pub kind: EntryKind,
    pub row: i64,
    pub unique_id: String,
    pub notes: String,
    /// The lowercase `TransactionType` value.
    pub transaction_type: &'static str,
    pub spot_price: Dec,
    pub crypto_in: Option<Dec>,
    pub crypto_fee: Dec,
    pub fiat_fee: Dec,
    pub fiat_in_no_fee: Option<Dec>,
    pub fiat_in_with_fee: Option<Dec>,
    pub crypto_out_no_fee: Option<Dec>,
    pub crypto_out_with_fee: Option<Dec>,
    pub fiat_out_no_fee: Option<Dec>,
    pub fiat_out_with_fee: Option<Dec>,
    pub crypto_sent: Option<Dec>,
    pub crypto_received: Option<Dec>,
    pub crypto_balance_change: Dec,
    pub crypto_taxable_amount: Dec,
    pub fiat_taxable_amount: Dec,
    /// `null` when evaluating it raises (RP2 evaluates it lazily).
    pub is_taxable: Option<bool>,
    pub is_earn: bool,
}

/// The response to a successful `check_entry`.
#[derive(Clone, Debug, Serialize)]
pub struct CheckEntryResponse {
    pub schema_version: u32,
    pub ok: bool,
    pub entry: DerivedEntry,
}

/// Identifies a taxable event by kind and row.
#[derive(Clone, Debug, Serialize)]
pub struct EventRef {
    pub kind: EntryKind,
    pub row: i64,
}

/// One `GainLoss`.
#[derive(Clone, Debug, Serialize)]
pub struct GainLossOut {
    pub event: EventRef,
    /// The acquired lot's row; `null` for earn receipts.
    pub lot: Option<i64>,
    pub crypto_amount: Dec,
    pub fiat_cost_basis: Dec,
    /// `taxable_event_fiat_amount_with_fee_fraction`.
    pub proceeds: Dec,
    pub fiat_gain: Dec,
    pub unit_cost_basis_override: Option<Dec>,
    pub long_term: bool,
    /// The Austrian disposal category, on Austrian books only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_category: Option<String>,
    /// Why the Austrian classification failed, on Austrian books only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_category_error: Option<ErrorBody>,
}

/// One `YearlyGainLoss`.
#[derive(Clone, Debug, Serialize)]
pub struct YearlyOut {
    pub year: i32,
    pub transaction_type: &'static str,
    pub long_term: bool,
    pub crypto_amount: Dec,
    pub fiat_amount: Dec,
    pub fiat_cost_basis: Dec,
    pub fiat_gain_loss: Dec,
}

/// One open-position lot.
#[derive(Clone, Debug, Serialize)]
pub struct OpenPositionOut {
    pub row: i64,
    pub sold_percentage: Dec,
    pub fiat_in_with_fee: Dec,
}

/// One account balance.
#[derive(Clone, Debug, Serialize)]
pub struct BalanceOut {
    pub exchange: String,
    pub holder: String,
    pub final_balance: Dec,
    pub acquired_balance: Dec,
    pub sent_balance: Dec,
    pub received_balance: Dec,
}

/// One asset's `ComputedData`.
#[derive(Clone, Debug, Serialize)]
pub struct AssetOutput {
    pub asset: String,
    /// Visible lot rows, in lot-list order.
    pub in_transactions: Vec<i64>,
    /// The effective acquisition basis of each lot in `in_transactions`.
    pub in_fiat_in_with_fee: Vec<Dec>,
    /// In `gain_loss_set` iteration order.
    pub gain_losses: Vec<GainLossOut>,
    /// In RP2's order (descending by `"<asset> <year> <LONG|SHORT> <type>"`).
    pub yearly: Vec<YearlyOut>,
    /// In lot-list order.
    pub open_positions: Vec<OpenPositionOut>,
    /// In RP2's order (by `"<exchange>_<holder>"`).
    pub balances: Vec<BalanceOut>,
}

/// The response to a successful `compute`, `compute_multi`, or `validate`.
#[derive(Clone, Debug, Serialize)]
pub struct ComputeResponse {
    pub schema_version: u32,
    pub ok: bool,
    /// `compute_multi` only: whether the Austrian runner ran. `false` (no
    /// swap pairs) means the caller computes each asset with `compute`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handled: Option<bool>,
    /// Per asset, in request order. Absent for `validate` and for an
    /// unhandled `compute_multi`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<Vec<AssetOutput>>,
}
