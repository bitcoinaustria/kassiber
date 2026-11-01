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
    pub(crate) fn value(&self) -> std::borrow::Cow<'_, Value> {
        match self {
            Self::Small(n) => std::borrow::Cow::Owned(Value::from(*n)),
            Self::Wide(n) => std::borrow::Cow::Borrowed(n),
        }
    }

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
}
