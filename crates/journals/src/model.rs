//! Rows before persistence. Decimal representation and optional-key presence matter.
use kassiber_pydecimal::Decimal;
use serde::{Serialize, Serializer};
use serde_json::Value;

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
pub struct JournalEntryRow {
    pub id: String,
    pub workspace_id: Value,
    pub profile_id: Value,
    pub transaction_id: Value,
    pub wallet_id: Value,
    pub account_id: Value,
    pub occurred_at: Value,
    pub entry_type: &'static str,
    pub asset: String,
    pub quantity: JournalDecimal,
    pub fiat_value: JournalDecimal,
    pub unit_cost: JournalDecimal,
    pub cost_basis: Option<JournalDecimal>,
    pub proceeds: Option<JournalDecimal>,
    pub gain_loss: Option<JournalDecimal>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_category: Option<String>,
    // Some(None) preserves the explicit null written for unmapped categories.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_kennzahl: Option<Option<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capital_gains_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculation: Option<Calculation>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Calculation {
    pub schema_version: u32,
    pub engine: &'static str,
    pub method: String,
    pub fragments: Vec<Fragment>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Fragment {
    pub quantity_msat: Value,
    pub quantity_msat_exact: String,
    pub cost_basis_exact: String,
    pub proceeds_exact: String,
    pub gain_loss_exact: String,
    pub unit_basis_override_exact: Option<String>,
    pub basis_authority: &'static str,
    pub acquisition_basis_exact: Option<String>,
    pub acquisition_quantity_msat: Option<Value>,
    pub acquisition_quantity_msat_exact: Option<String>,
    pub event: Source,
    pub lot: Option<Source>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Source {
    #[serde(flatten, serialize_with = "serialize_provenance")]
    pub provenance: std::sync::Arc<SourceProvenance>,
    pub asset: String,
    pub spot_price_exact: String,
    pub fiat_fee_exact: String,
    pub crypto_fee_msat: Value,
    pub crypto_fee_msat_exact: String,
}
fn serialize_provenance<S: Serializer>(
    provenance: &std::sync::Arc<SourceProvenance>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    provenance.as_ref().serialize(serializer)
}
#[derive(Clone, Debug, Serialize)]
pub struct SourceProvenance {
    pub transaction_id: Value,
    pub engine_event_id: String,
    pub occurred_at: String,
    pub pricing: PricingFacts,
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
pub struct PricingFacts(pub [Value; 12]);
impl Serialize for PricingFacts {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(12))?;
        for (key, value) in FRAGMENT_PRICING_FIELDS.iter().zip(&self.0) {
            map.serialize_entry(key, value)?;
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
    pub quantity: f64,
    pub quantity_msat: Value,
    pub proceeds: f64,
    pub cost_basis: f64,
    pub gain_loss: f64,
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
    pub entries: Vec<JournalEntryRow>,
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
