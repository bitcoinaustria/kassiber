//! Rows before persistence. Decimal representation and optional-key presence matter.
use kassiber_pydecimal::Decimal;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::borrow::Cow;

#[derive(Clone, Debug)]
pub struct JournalDecimal(pub Decimal);
impl Serialize for JournalDecimal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("$decimal", &self.0.to_string())?;
        map.end()
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum JournalQuantity {
    Finite(JournalDecimal),
    NonFinite {
        #[serde(rename = "$decimal")]
        text: NonFiniteDecimal,
    },
}
#[derive(Clone, Copy, Debug, Serialize)]
pub enum NonFiniteDecimal {
    #[serde(rename = "Infinity")]
    PositiveInfinity,
    #[serde(rename = "-Infinity")]
    NegativeInfinity,
    NaN,
}
impl From<JournalDecimal> for JournalQuantity {
    fn from(value: JournalDecimal) -> Self {
        Self::Finite(value)
    }
}
impl std::fmt::Display for JournalQuantity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Finite(value) => value.0.fmt(f),
            Self::NonFinite { text } => f.write_str(match text {
                NonFiniteDecimal::PositiveInfinity => "Infinity",
                NonFiniteDecimal::NegativeInfinity => "-Infinity",
                NonFiniteDecimal::NaN => "NaN",
            }),
        }
    }
}
impl JournalQuantity {
    pub(crate) fn from_audit(value: f64, outbound: bool) -> Result<Self, crate::Refusal> {
        if value.is_finite() {
            let quantity = kassiber_pyjson::float_repr(value).parse::<Decimal>()?;
            Ok(JournalDecimal(if outbound {
                quantity.neg(&kassiber_pydecimal::Context::RP2)?
            } else {
                quantity
            })
            .into())
        } else {
            Ok(Self::NonFinite {
                text: if value.is_nan() {
                    NonFiniteDecimal::NaN
                } else if value.is_sign_negative() != outbound {
                    NonFiniteDecimal::NegativeInfinity
                } else {
                    NonFiniteDecimal::PositiveInfinity
                },
            })
        }
    }
    pub(crate) fn msat(&self) -> Result<Value, crate::Refusal> {
        match self {
            Self::Finite(value) => crate::render::msat(&value.0),
            Self::NonFinite {
                text: NonFiniteDecimal::NaN,
            } => Err(crate::Refusal::new(
                "ValueError",
                "cannot convert NaN to integer",
            )),
            Self::NonFinite { .. } => Err(crate::Refusal::new(
                "OverflowError",
                "cannot convert Infinity to integer",
            )),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct JournalEntryRow<'a> {
    pub id: String,
    pub workspace_id: &'a Value,
    pub profile_id: &'a Value,
    pub transaction_id: Value,
    pub wallet_id: &'a Value,
    pub account_id: &'a Value,
    pub occurred_at: Value,
    pub entry_type: &'static str,
    pub asset: Cow<'a, str>,
    pub quantity: JournalQuantity,
    pub fiat_value: JournalDecimal,
    pub unit_cost: JournalDecimal,
    pub cost_basis: Option<JournalDecimal>,
    pub proceeds: Option<JournalDecimal>,
    pub gain_loss: Option<JournalDecimal>,
    pub description: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_category: Option<String>,
    // Some(None) preserves the explicit null written for unmapped categories.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_kennzahl: Option<Option<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capital_gains_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculation: Option<Calculation<'a>>,
}
#[derive(Clone, Debug, Serialize)]
pub struct TransferAudit {
    #[serde(flatten)]
    pub facts: Value,
    #[serde(serialize_with = "serialize_float")]
    pub crypto_sent: f64,
    #[serde(serialize_with = "serialize_float")]
    pub crypto_received: f64,
    #[serde(serialize_with = "serialize_float")]
    pub crypto_fee: f64,
    #[serde(serialize_with = "serialize_float")]
    pub spot_price: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Calculation<'a> {
    pub schema_version: u32,
    pub engine: &'static str,
    pub method: Cow<'a, str>,
    pub fragments: Vec<Fragment<'a>>,
    /// `false` for a gift's loss the country does not count; absent
    /// otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss_deductible: Option<bool>,
    /// Why a sale's proceeds are not its price (`market_value`,
    /// `seventy_percent`); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valuation: Option<&'static str>,
}
impl Calculation<'_> {
    /// Whether the entry has none of a rules country's own facts, so it may
    /// take the compact encoding; with one (as with a fragment's
    /// `expenses_exact`) it keeps the legacy JSON form.
    pub(crate) fn plain(&self) -> bool {
        self.loss_deductible.is_none() && self.valuation.is_none()
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum JournalMsat {
    Small(i64),
    Wide(Box<Value>),
}
impl From<Value> for JournalMsat {
    fn from(value: Value) -> Self {
        Self::Wide(Box::new(value))
    }
}
impl JournalMsat {
    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Small(n) => Some(*n),
            Self::Wide(n) => n.as_i64(),
        }
    }
    pub(crate) fn into_value(self) -> Value {
        match self {
            Self::Small(n) => Value::from(n),
            Self::Wide(n) => *n,
        }
    }
}
impl std::fmt::Display for JournalMsat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Small(n) => n.fmt(f),
            Self::Wide(n) => n.fmt(f),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Fragment<'a> {
    pub quantity_msat: JournalMsat,
    pub quantity_msat_exact: String,
    pub cost_basis_exact: Cow<'static, str>,
    pub proceeds_exact: Cow<'static, str>,
    pub gain_loss_exact: Cow<'static, str>,
    /// Costs the engine deducted from the gain (Germany's Werbungskosten);
    /// only for a country whose rules give them, which stores the fragment
    /// in the legacy JSON shape instead of the compact tuple.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expenses_exact: Option<Cow<'static, str>>,
    pub unit_basis_override_exact: Option<Cow<'static, str>>,
    pub basis_authority: &'static str,
    pub acquisition_basis_exact: Option<Cow<'static, str>>,
    pub acquisition_quantity_msat: Option<JournalMsat>,
    pub acquisition_quantity_msat_exact: Option<String>,
    #[serde(serialize_with = "serialize_source")]
    pub event: std::sync::Arc<Source<'a>>,
    #[serde(serialize_with = "serialize_optional_source")]
    pub lot: Option<std::sync::Arc<Source<'a>>>,
}
fn serialize_source<S: Serializer>(
    source: &std::sync::Arc<Source<'_>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    source.as_ref().serialize(serializer)
}
fn serialize_optional_source<S: Serializer>(
    source: &Option<std::sync::Arc<Source<'_>>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    source.as_deref().serialize(serializer)
}
#[derive(Clone, Debug, Serialize)]
pub struct Source<'a> {
    #[serde(flatten, serialize_with = "serialize_provenance")]
    pub provenance: std::sync::Arc<SourceProvenance<'a>>,
    pub asset: Cow<'a, str>,
    pub spot_price_exact: Cow<'static, str>,
    pub fiat_fee_exact: Cow<'static, str>,
    pub crypto_fee_msat: JournalMsat,
    pub crypto_fee_msat_exact: Cow<'static, str>,
}
fn serialize_provenance<S: Serializer>(
    provenance: &std::sync::Arc<SourceProvenance<'_>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    provenance.as_ref().serialize(serializer)
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceProvenance<'a> {
    pub transaction_id: Value,
    pub engine_event_id: Cow<'a, str>,
    pub occurred_at: String,
    pub pricing: PricingFacts<'a>,
}
pub const FRAGMENT_PRICING_FIELDS: [&str; 12] = [
    "fiat_currency",
    "fiat_rate_exact",
    "fiat_value_exact",
    "pricing_source_kind",
    "pricing_quality",
    "fiat_price_source",
    "pricing_timestamp",
    "pricing_provider",
    "pricing_pair",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
];
#[derive(Clone, Debug)]
pub enum PricingFacts<'a> {
    Borrowed([&'a Value; 12]),
    Owned(Box<[Value; 12]>),
}
impl PricingFacts<'_> {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Value> {
        (0..12).map(|index| &self[index])
    }
}
impl std::ops::Index<usize> for PricingFacts<'_> {
    type Output = Value;
    fn index(&self, index: usize) -> &Self::Output {
        match self {
            Self::Borrowed(values) => values[index],
            Self::Owned(values) => &values[index],
        }
    }
}
impl std::ops::IndexMut<usize> for PricingFacts<'_> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        if let Self::Borrowed(values) = self {
            *self = Self::Owned(Box::new(values.map(Clone::clone)));
        }
        let Self::Owned(values) = self else {
            unreachable!()
        };
        &mut values[index]
    }
}
impl Serialize for PricingFacts<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(12))?;
        match self {
            Self::Borrowed(values) => {
                for (key, value) in FRAGMENT_PRICING_FIELDS.iter().zip(values) {
                    map.serialize_entry(key, *value)?;
                }
            }
            Self::Owned(values) => {
                for (key, value) in FRAGMENT_PRICING_FIELDS.iter().zip(values.iter()) {
                    map.serialize_entry(key, value)?;
                }
            }
        }
        map.end()
    }
}
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct QuarantineRow {
    pub transaction_id: Value,
    pub workspace_id: Value,
    pub profile_id: Value,
    pub reason: String,
    pub detail_json: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct TaxSummaryRow {
    pub year: i32,
    pub asset: String,
    pub transaction_type: &'static str,
    pub capital_gains_type: &'static str,
    #[serde(serialize_with = "serialize_float")]
    pub quantity: f64,
    pub quantity_msat: Value,
    #[serde(serialize_with = "serialize_float")]
    pub proceeds: f64,
    #[serde(serialize_with = "serialize_float")]
    pub cost_basis: f64,
    #[serde(serialize_with = "serialize_float")]
    pub gain_loss: f64,
}
fn serialize_float<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.is_finite() {
        serializer.serialize_f64(*value)
    } else {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("$float", &kassiber_pyjson::float_repr(*value))?;
        map.end()
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Holding {
    pub quantity: JournalDecimal,
    pub cost_basis: JournalDecimal,
}
/// Python dicts with tuple keys, retaining first appearance order.
#[derive(Clone, Debug, Serialize)]
pub struct Holdings {
    #[serde(rename = "$pairs")]
    pub pairs: Vec<(Vec<Value>, Holding)>,
}
#[derive(Clone, Debug, Serialize)]
pub struct JournalState<'a> {
    pub entries: Vec<JournalEntryRow<'a>>,
    pub quarantines: Vec<QuarantineRow>,
    pub cross_asset_pairs: &'a Value,
    pub direct_swap_payouts: Value,
    pub tax_summary: Vec<TaxSummaryRow>,
    pub account_holdings: Holdings,
    pub wallet_holdings: Holdings,
    pub ownership_review_counts: Value,
    pub ownership_review_candidates: Value,
    pub custody_component_blockers: &'a Value,
    pub custody_native_reconciliations: Value,
    pub custody_quantity: &'a Value,
    pub custody_transfers: &'a Value,
    pub quantity_differences: Value,
    pub latest_rates: &'a Value,
    pub warnings: &'a Value,
    pub custody_gap_ignored_transaction_ids: &'a Value,
    pub custody_gap_accounting_ignored_transaction_ids: &'a Value,
    /// Views of `custody_quantity`'s projection observations and postings,
    /// which `custody_quantity` then leaves out; `None` reads them from it.
    #[serde(skip)]
    pub quantity_views: Option<&'a kassiber_custody::quantity::runtime::QuantityViews<'a>>,
}
impl<'a> JournalState<'a> {
    /// The quantity projection's observations and postings.
    pub fn quantity_refs(&self) -> (Vec<ObservationRef<'a>>, Vec<PostingRef<'a>>) {
        quantity_refs(self.custody_quantity, self.quantity_views)
    }
}

/// The observations and postings of `quantity["projection"]`, from `views`
/// when given.
pub(crate) fn quantity_refs<'a>(
    quantity: &'a Value,
    views: Option<&'a kassiber_custody::quantity::runtime::QuantityViews<'a>>,
) -> (Vec<ObservationRef<'a>>, Vec<PostingRef<'a>>) {
    match views {
        Some(views) => (
            views
                .observations
                .iter()
                .map(ObservationRef::View)
                .collect(),
            views.postings.iter().map(PostingRef::View).collect(),
        ),
        None => {
            let projection = &quantity["projection"];
            let list = |key: &str| projection[key].as_array().map(Vec::as_slice).unwrap_or(&[]);
            (
                list("observations")
                    .iter()
                    .map(ObservationRef::Json)
                    .collect(),
                list("postings").iter().map(PostingRef::Json).collect(),
            )
        }
    }
}

/// A quantity projection observation: its JSON object, or a view of it.
#[derive(Clone, Copy)]
pub enum ObservationRef<'a> {
    Json(&'a Value),
    View(&'a kassiber_custody::quantity::runtime::ObservationView<'a>),
}
impl<'a> ObservationRef<'a> {
    /// The field `key`, or null.
    pub fn at(self, key: &str) -> &'a Value {
        match self {
            Self::Json(o) => &o[key],
            Self::View(o) => o.at(key),
        }
    }
    /// Whether this is JSON null, the stand-in for a missing observation.
    pub fn is_null(self) -> bool {
        matches!(self, Self::Json(Value::Null))
    }
}
impl std::ops::Index<&str> for ObservationRef<'_> {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        (*self).at(key)
    }
}
impl std::fmt::Debug for ObservationRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Observation")
            .field(&self.at("quantity_hash"))
            .finish()
    }
}

/// A quantity projection posting: its JSON object, or a view of it.
#[derive(Clone, Copy)]
pub enum PostingRef<'a> {
    Json(&'a Value),
    View(&'a kassiber_custody::quantity::PostingView<'a>),
}
impl<'a> PostingRef<'a> {
    /// The field `key`, or null. A view read in place has no `$type` or
    /// `domain`, which no journal reads.
    pub fn at(self, key: &str) -> &'a Value {
        match self {
            Self::Json(p) => &p[key],
            Self::View(p) => p.at(key),
        }
    }
}
impl std::fmt::Debug for PostingRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Posting")
            .field(&self.at("posting_id"))
            .finish()
    }
}
