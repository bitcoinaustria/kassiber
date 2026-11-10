//! The JSON boundary, schema version 1: request and response documents.
//!
//! Decimals cross the boundary in the canonical form `[-]<digits>E<exp>`
//! ([`Dec`]), which keeps the coefficient and exponent. Requests reject
//! unknown fields so a drifting caller fails loudly instead of being
//! half-understood. `docs/reference/tax-engine.md` documents the format.

use std::fmt;

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::decimal::Decimal;
use crate::error::ErrorClass;
use crate::ledger::request::Direction;

/// The only schema version this engine reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// A decimal in the canonical boundary form.
#[derive(Clone, Debug)]
pub struct Dec(pub Decimal);

impl Serialize for Dec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // The canonical text needs no escaping, so this writes what
        // `serialize_str` of `to_canonical()` writes, without the string.
        serializer.collect_str(&Canonical(&self.0))
    }
}

/// [`Decimal::to_canonical`] as a `Display`.
struct Canonical<'a>(&'a Decimal);

impl fmt::Display for Canonical<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.write_canonical(f)
    }
}

impl<'de> Deserialize<'de> for Dec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_string(DecVisitor)
    }
}

/// Reads a canonical decimal from a string as `String::deserialize` reads
/// the string, without keeping it.
struct DecVisitor;

impl de::Visitor<'_> for DecVisitor {
    type Value = Dec;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `String`'s own expectation, so a value of another type fails
        // with its message.
        formatter.write_str("a string")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Dec, E> {
        Decimal::from_canonical(text)
            .map(Dec)
            .map_err(|_| E::custom(format!("not a canonical decimal: {text:?}")))
    }

    fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Dec, E> {
        match std::str::from_utf8(bytes) {
            Ok(text) => self.visit_str(text),
            Err(_) => Err(E::invalid_value(de::Unexpected::Bytes(bytes), &self)),
        }
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
    /// What a rules country's ledger walk adds; never read from a request.
    #[serde(skip)]
    pub country: CountryFacts,
}

/// The facts a rules country's ledger walk gives an entry beyond RP2's
/// arguments (`docs/reference/tax-engine.md#rules-countries`). Every other
/// entry has the defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryFacts {
    /// An acquisition at zero cost without income (the country's
    /// `zero_cost_kinds`): its price and basis are zero.
    pub zero_cost: bool,
    /// Why a sale's value is not its price (the country's `below_market`):
    /// `market_value` or `seventy_percent`.
    pub valuation: Option<&'static str>,
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
    /// The United States: long-term once held more than one year by UTC
    /// calendar date (`docs/reference/tax-country-us.md`).
    Us {},
    /// A country whose rules the registry states one by one.
    Rules(RulesSpec),
}

/// The rules of a registry country (`docs/reference/tax-countries.md`): each
/// field is one shared engine feature, and an absent one is the generic
/// rule, so a country names only what it changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesSpec {
    /// The IANA zone whose local dates count holding periods (a zone of
    /// the registry's calendar).
    pub timezone: String,
    pub holding: Holding,
    /// The tax year's first day, `MM-DD` in the zone; the calendar year
    /// (`01-01`) when absent. The yearly totals and the tax summary are by
    /// the tax year it starts, named by its first day's year.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tax_year_start: Option<String>,
    /// The only pool scope the country allows (Germany's per-wallet order of
    /// use, the United Kingdom's single pool); any when absent. A request
    /// on the other scope fails rather than compute the wrong pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_scope: Option<PoolScope>,
    /// What paying a fee in the asset is.
    #[serde(default, skip_serializing_if = "FeeUnits::is_basis")]
    pub fee_units: FeeUnits,
    /// The values of a taxable exchange of one asset for another.
    #[serde(default, skip_serializing_if = "SwapValues::is_own")]
    pub swap_values: SwapValues,
    /// What giving an asset away is (an outbound kind with `gift`).
    #[serde(default, skip_serializing_if = "Given::is_quarantine")]
    pub gifts_given: Given,
    /// What donating an asset is (an outbound kind with `donat`).
    #[serde(default, skip_serializing_if = "Given::is_quarantine")]
    pub donations_given: Given,
    /// What receiving a gift is (an inbound kind with `gift`).
    #[serde(default, skip_serializing_if = "Received::is_acquisition")]
    pub gifts_received: Received,
    /// Whether a gift's or donation's loss counts.
    #[serde(default, skip_serializing_if = "GiftLosses::is_deductible")]
    pub gift_losses: GiftLosses,
    /// Kinds the country does not book until a reviewer settles them
    /// (Germany's unclassified airdrops, a hard fork's cost split, Spain's
    /// lost coins): each such row is quarantined with the country's code,
    /// and taints the asset's later disposals.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused_receipts: Vec<RefusedReceipt>,
    /// Inbound kinds booked as acquisitions at zero cost, with no income
    /// (ledger only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zero_cost_kinds: Vec<String>,
    /// What a sale or payment below the market value its row records is
    /// worth (ledger only).
    #[serde(default, skip_serializing_if = "BelowMarket::is_price")]
    pub below_market: BelowMarket,
}

/// Kinds a country refuses, with its fail-closed code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefusedReceipt {
    /// `inbound` (absent): normalized kinds, compared exactly, held as
    /// `acquisition_valuation_unsupported`. `outbound`: tokens of the row's
    /// kind (`lost` matches `lost_keys`), each a non-sale disposal token,
    /// held as `non_sale_disposal_kind`.
    #[serde(default, skip_serializing_if = "Direction::is_inbound")]
    pub direction: Direction,
    pub kinds: Vec<String>,
    /// The country's code (`airdrop_classification_required`).
    pub code: String,
    /// Why, for the quarantine's detail.
    pub detail: String,
}

impl RulesSpec {
    /// The refusal of a row of `kind` in `direction`, if the country has one.
    pub fn refused_receipt(&self, direction: Direction, kind: &str) -> Option<&RefusedReceipt> {
        self.refused_receipts.iter().find(|refused| {
            refused.direction == direction
                && match direction {
                    Direction::Inbound => refused.kinds.iter().any(|k| k == kind),
                    // `kind_has_token`'s test.
                    Direction::Outbound => {
                        !kind.is_empty() && refused.kinds.iter().any(|k| kind.contains(k.as_str()))
                    }
                }
        })
    }
}

/// What a sale or payment below market value is worth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BelowMarket {
    /// The row's own value (the generic rule).
    #[default]
    Price,
    /// The market value when it is higher (Spain, art. 35.2 LIRPF).
    MarketValue,
    /// 70 % of the market value when the value is below that; exactly 70 %
    /// is not below (Japan, NTA FAQ 2-10).
    SeventyPercent,
}

impl BelowMarket {
    /// Whether this is the generic rule.
    pub fn is_price(&self) -> bool {
        *self == BelowMarket::Price
    }
}

/// What an outbound gift or donation is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Given {
    /// Quarantined as `non_sale_disposal_kind` (the generic rule).
    #[default]
    Quarantine,
    /// A disposal at its value (Spain, Ireland, Japan, ...).
    MarketValue,
}

impl Given {
    /// Whether this is the generic rule.
    pub fn is_quarantine(&self) -> bool {
        *self == Given::Quarantine
    }
}

/// What an inbound gift is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Received {
    /// An acquisition at its value (the generic rule).
    #[default]
    Acquisition,
    /// Quarantined as `acquisition_valuation_unsupported`: the donor's
    /// acquisition date and cost carry over, and the book has neither
    /// (Germany's § 23 Abs. 1 Satz 3 EStG).
    Quarantine,
}

impl Received {
    /// Whether this is the generic rule.
    pub fn is_acquisition(&self) -> bool {
        *self == Received::Acquisition
    }
}

/// Whether the loss of a gift or donation counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GiftLosses {
    #[default]
    Deductible,
    /// The loss is reported but marked not deductible (Spain, art. 33.5.c
    /// LIRPF).
    NotDeductible,
}

impl GiftLosses {
    /// Whether this is the generic rule.
    pub fn is_deductible(&self) -> bool {
        *self == GiftLosses::Deductible
    }
}

/// The values of a taxable cross-asset exchange: the proceeds of the asset
/// given and the cost of the asset received. When one leg has no value,
/// every mode but `own` takes the other leg's for both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwapValues {
    /// Each leg at its own value (the generic rule).
    #[default]
    Own,
    /// Proceeds are the received asset's value, and its cost the given
    /// asset's (Germany, the United Kingdom).
    ReceivedGiven,
    /// Proceeds and cost are the received asset's value (Ireland, Japan).
    Received,
    /// Proceeds and cost are the higher of the two values (Spain).
    Higher,
}

impl SwapValues {
    /// Whether this is the generic rule.
    pub fn is_own(&self) -> bool {
        *self == SwapValues::Own
    }
}

/// What fee units paid in the asset are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeUnits {
    /// Kassiber's generic rule: their basis leaves the book without a gain
    /// (a fee entry's proceeds are its basis), and a sale's fee units are
    /// part of the sale.
    #[default]
    Basis,
    /// Disposals at their value (the sale's spot), with a gain or loss; a
    /// sale's fee units stay part of the sale, whose proceeds are its price
    /// (Spain's and Ireland's single row).
    Disposal,
    /// Disposals at their value, and a sale's fee units are a disposal of
    /// their own, taken before the sale's other units; the sale's fragments
    /// carry the fee's value as expenses, split by quantity (Germany's
    /// Werbungskosten, the United Kingdom's allowable cost).
    DisposalBeforeSale,
}

impl FeeUnits {
    /// Whether this is the generic rule.
    pub fn is_basis(&self) -> bool {
        *self == FeeUnits::Basis
    }

    /// Whether fee entries realize their gain.
    pub fn realize(self) -> bool {
        self != FeeUnits::Basis
    }
}

/// How the holding period divides gains.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Holding {
    /// No holding period: every gain is short-term.
    Never,
    /// Long-term once held more than one year by local date: the period
    /// runs from the day after acquisition through the disposal's day, so
    /// a disposal on the anniversary is still short-term, and 29 February's
    /// anniversary is 28 February (the United States' rule, and Germany's
    /// one-year period read the other way round).
    MoreThanOneYear,
}

/// Where a disposal may take its lots from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolScope {
    /// Every lot of the asset, whatever wallet holds it (RP2's universal
    /// application).
    #[default]
    Global,
    /// Only the lots the disposing wallet holds; a move carries the lots it
    /// takes, with their basis and acquisition time, to its destination.
    Wallet,
}

impl PoolScope {
    /// Whether this is the default, universal scope.
    pub fn is_global(&self) -> bool {
        *self == PoolScope::Global
    }
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
    /// Germany's FIFO holding period at the wallet's moving average cost (a
    /// matching method, `crate::matching`).
    MovingAverageFifoPeriod,
    /// The United Kingdom's share matching: same day, then the next 30
    /// days, then the section 104 pool (a matching method,
    /// `crate::share_matching`).
    UkShareMatching,
    /// Japan's total average (総平均法) per tax year (a matching method).
    JpTotalAverage,
    /// Japan's moving average (移動平均法), the book value left after a
    /// disposal rounded up to the yen (a matching method).
    JpMovingAverage,
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
            MethodName::MovingAverageFifoPeriod => "moving_average_fifo_period",
            MethodName::UkShareMatching => "uk_share_matching",
            MethodName::JpTotalAverage => "jp_total_average",
            MethodName::JpMovingAverage => "jp_moving_average",
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
    /// `global` when absent.
    #[serde(default)]
    pub pool_scope: PoolScope,
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
    /// Costs of the disposal deducted from its gain: a sale's fee value
    /// when its fee units are a disposal of their own; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expenses: Option<Dec>,
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
    /// The sum of the fragments' expenses, when any has them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expenses: Option<Dec>,
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
    /// Under the wallet pool scope, each wallet's remaining units and their
    /// own basis, by exchange label; absent under the global scope, whose
    /// holdings are an allocation at the asset's average basis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_holdings: Option<Vec<WalletHoldingOut>>,
}

/// One wallet's remaining units of an asset under the wallet pool scope.
#[derive(Clone, Debug, Serialize)]
pub struct WalletHoldingOut {
    pub exchange: String,
    pub holder: String,
    pub quantity: Dec,
    pub cost_basis: Dec,
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
