//! The ledger response, schema version 2 (design §3.7, §3.9).
//!
//! A success lists the journal ledger the reader
//! (`kassiber/core/engines/ledger_response.py`) renders into Kassiber's
//! `TaxEngineLedgerResult`: entries, the engine's quarantines, the transfer
//! audit, the tax summary, holdings, and the rows the generic carry
//! rewrote. Values are exact decimals in the canonical boundary form; the
//! reader alone performs the legacy float conversions, JSON serialization,
//! and uuid generation (design §2). Every list is in the order the reader
//! must keep.
//!
//! A failure names the stage that failed and the error raised there; the
//! reader raises exactly what the phase-3 adapter raised at that stage,
//! wrapping it where the adapter wrapped it.
//!
//! The `prepare` operation answers with the intermediate stages instead
//! (design §3.10), per asset, so a test can compare each stage with the
//! oracle's own intermediates: so far the normalized stage.

use serde::ser::Serializer;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::decimal::Decimal;
use crate::error::{EngineError, ErrorClass};
use crate::manifest::{DetailKind, EntryType, Manifest, QuarantineReason};
use crate::model::Dec;

use super::request::{Regime, SCHEMA_VERSION};

/// Where a ledger run failed (design §3.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// An entry constructor, in walk order: raised unwrapped.
    Prepare,
    /// The Austrian swap-pair checks: wrapped as
    /// `RP2 cross-asset input validation failed: ...`.
    Validate,
    /// The multi-asset hook, including the method gate: wrapped as
    /// `RP2 multi-asset tax calculation failed: ...`.
    ComputeMulti,
    /// One asset's computation: wrapped as
    /// `RP2 tax calculation failed for asset '<asset>': ...`.
    Compute,
    /// An Austrian classification, at the first failing fragment: raised
    /// unwrapped.
    Classify,
    /// A malformed request.
    Request,
    /// An operation or capability the engine does not implement yet.
    Unsupported,
}

/// The class of the error a stage raised: an engine class, or the
/// adapter's own `AppError` (the method gate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InnerClass {
    Engine(ErrorClass),
    AppError,
}

impl Serialize for InnerClass {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            InnerClass::Engine(class) => class.serialize(serializer),
            InnerClass::AppError => serializer.serialize_str("AppError"),
        }
    }
}

/// The error a stage raised.
#[derive(Clone, Debug, Serialize)]
pub struct Inner {
    pub class: InnerClass,
    pub message: String,
}

/// A failed run: the stage, the asset for `compute`, and the error.
#[derive(Clone, Debug, Serialize)]
pub struct StagedError {
    pub stage: Stage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    pub inner: Inner,
}

impl StagedError {
    /// `error`, raised at `stage`.
    pub fn new(stage: Stage, error: EngineError) -> Self {
        StagedError {
            stage,
            asset: None,
            inner: Inner {
                class: InnerClass::Engine(error.class),
                message: error.message,
            },
        }
    }

    /// A request the engine could not read or does not implement.
    pub fn contract(error: EngineError) -> Self {
        let stage = if error.class == ErrorClass::Unsupported {
            Stage::Unsupported
        } else {
            Stage::Request
        };
        StagedError::new(stage, error)
    }
}

/// The response to a failed schema-2 request.
#[derive(Clone, Debug, Serialize)]
pub struct ErrorResponse {
    pub schema_version: u32,
    pub ok: bool,
    pub error: StagedError,
}

impl ErrorResponse {
    pub fn new(error: StagedError) -> Self {
        ErrorResponse {
            schema_version: SCHEMA_VERSION,
            ok: false,
            error,
        }
    }
}

/// The response to a `manifest` request: the manifest's fields beside the
/// envelope.
#[derive(Clone, Debug, Serialize)]
pub struct ManifestResponse {
    pub schema_version: u32,
    pub ok: bool,
    #[serde(flatten)]
    pub manifest: Manifest,
}

/// The response to a successful `ledger` request.
#[derive(Clone, Debug, Serialize)]
pub struct LedgerResponse {
    pub schema_version: u32,
    pub ok: bool,
    pub engine_version: &'static str,
    /// Assets in first-appearance order; within one: acquisitions in lot
    /// order, realized entries in first-key order, then transfer legs in
    /// audit order.
    pub entries: Vec<EntryOut>,
    /// Rail carry, then Austrian swap pairs, then per asset (normalize,
    /// then gates).
    pub quarantines: Vec<QuarantineOut>,
    pub intra_audit: Vec<AuditOut>,
    pub tax_summary: Vec<SummaryOut>,
    pub holdings: Vec<HoldingOut>,
    pub carried_rows: Vec<CarriedRow>,
}

/// One journal entry.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum EntryOut {
    Acquisition(AcquisitionOut),
    Realized(RealizedOut),
    TransferLeg(TransferLegOut),
}

/// An `acquisition` entry.
#[derive(Clone, Debug, Serialize)]
pub struct AcquisitionOut {
    pub entry_type: EntryType,
    pub row: String,
    /// The wallet's label.
    pub wallet: String,
    pub asset: String,
    pub quantity: Dec,
    /// The effective basis.
    pub fiat_value: Dec,
    pub unit_cost: Dec,
    pub description: String,
}

/// An `income`, `disposal`, `fee`, or `transfer_fee` entry.
#[derive(Clone, Debug, Serialize)]
pub struct RealizedOut {
    pub entry_type: EntryType,
    /// The taxable event's row; a transfer fee's out row.
    pub row: String,
    pub wallet: String,
    pub asset: String,
    /// Signed.
    pub quantity: Dec,
    pub proceeds: Dec,
    pub cost_basis: Dec,
    pub gain_loss: Dec,
    pub description: String,
    /// After overrides.
    pub at_category: Option<String>,
    /// `long` or `short`.
    pub capital_gains_type: &'static str,
    pub fragments: Vec<FragmentOut>,
}

/// A `transfer_out` or `transfer_in` entry, rendered from audit row `audit`.
#[derive(Clone, Debug, Serialize)]
pub struct TransferLegOut {
    pub entry_type: EntryType,
    pub audit: usize,
}

/// One gain/loss fragment of a realized entry.
#[derive(Clone, Debug, Serialize)]
pub struct FragmentOut {
    pub crypto_amount: Dec,
    pub cost_basis: Dec,
    pub proceeds: Dec,
    pub gain_loss: Dec,
    pub unit_basis_override: Option<Dec>,
    pub event: FragmentEvent,
    /// `null` for an earn receipt.
    pub lot: Option<FragmentLot>,
}

/// The taxable event of a fragment.
#[derive(Clone, Debug, Serialize)]
pub struct FragmentEvent {
    pub row: String,
    pub spot_price: Dec,
    pub fiat_fee: Dec,
    pub crypto_fee: Dec,
}

/// The acquired lot of a fragment.
#[derive(Clone, Debug, Serialize)]
pub struct FragmentLot {
    pub row: String,
    pub spot_price: Dec,
    pub fiat_fee: Dec,
    pub crypto_fee: Dec,
    /// The effective basis.
    pub basis: Dec,
    pub crypto_in: Dec,
}

/// One quarantine the engine raised.
#[derive(Clone, Debug, Serialize)]
pub struct QuarantineOut {
    /// The row whose anchor the quarantine names.
    pub row: String,
    pub reason: QuarantineReason,
    /// The template that built `detail`; the reader adds row facts by it.
    pub detail_kind: DetailKind,
    /// Typed JSON. A float the adapter wrote is `{"float": "<canonical>"}`.
    pub detail: Map<String, Value>,
}

/// Per-regime msat of a mixed Austrian move.
#[derive(Clone, Debug, Serialize)]
pub struct Flows {
    pub alt: u64,
    pub neu: u64,
}

/// What left the source and what reached the destination, per regime.
#[derive(Clone, Debug, Serialize)]
pub struct RegimeFlows {
    pub out: Flows,
    #[serde(rename = "in")]
    pub inbound: Flows,
}

/// One booked move.
#[derive(Clone, Debug, Serialize)]
pub struct AuditOut {
    /// The index into the request's `intra_pairs`.
    pub pair: usize,
    pub sent: Dec,
    pub received: Dec,
    pub fee: Dec,
    pub sent_msat: u64,
    pub received_msat: u64,
    pub fee_msat: u64,
    pub spot_price: Option<Dec>,
    pub at_regime: Option<String>,
    pub regime_flows: Option<RegimeFlows>,
    pub at_pool: Option<String>,
    /// The transfer legs' description.
    pub description: String,
}

/// One tax-summary row.
#[derive(Clone, Debug, Serialize)]
pub struct SummaryOut {
    /// The event's year in its own offset.
    pub year: i32,
    pub asset: String,
    pub transaction_type: &'static str,
    pub long_term: bool,
    pub quantity: Dec,
    pub proceeds: Dec,
    pub cost_basis: Dec,
    pub gain_loss: Dec,
}

/// One positive balance, in balance order.
#[derive(Clone, Debug, Serialize)]
pub struct HoldingOut {
    pub asset: String,
    /// The wallet's label.
    pub wallet: String,
    pub quantity: Dec,
    pub cost_basis: Dec,
}

/// A row the generic carry rewrote: its carried value and rate as fixed
/// strings.
#[derive(Clone, Debug, Serialize)]
pub struct CarriedRow {
    pub row: String,
    pub value: String,
    pub rate: Option<String>,
}

/// The response to a successful `prepare` request: each asset's stages
/// after the final prepare pass, assets in first-appearance order.
#[derive(Clone, Debug, Serialize)]
pub struct PrepareResponse {
    pub schema_version: u32,
    pub ok: bool,
    pub engine_version: &'static str,
    pub assets: Vec<PreparedAssetOut>,
}

/// One asset's stages.
#[derive(Clone, Debug, Serialize)]
pub struct PreparedAssetOut {
    pub asset: String,
    pub normalized: NormalizedOut,
}

/// One asset, normalized (the adapter's `NormalizedTaxAssetInputs`).
#[derive(Clone, Debug, Serialize)]
pub struct NormalizedOut {
    pub events: Vec<EventOut>,
    pub transfers: Vec<TransferOut>,
    /// Events and moves in the order normalize met them: `["event", row]`
    /// or `["transfer", out row]`.
    pub ordered_items: Vec<(&'static str, String)>,
    /// Normalize's quarantines, in order.
    pub quarantines: Vec<QuarantineOut>,
    /// `occurred_at` of the earliest row whose transaction a quarantine
    /// names, compared as text.
    pub earliest_lot_contamination_at: Option<String>,
}

/// A price or fiat value of the normalized stage. A stored rate or value
/// may say `Infinity`, which Python's `Decimal` takes for a positive price.
#[derive(Clone, Debug, PartialEq)]
pub enum Fiat {
    Finite(Decimal),
    Infinity,
}

impl Serialize for Fiat {
    /// The canonical form, or CPython's `Infinity`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Fiat::Finite(value) => serializer.serialize_str(&value.to_canonical()),
            Fiat::Infinity => serializer.serialize_str("Infinity"),
        }
    }
}

/// An event the walk books: an acquisition, income, or disposal
/// (`NormalizedTaxEvent`). The row's id, time, wallet id, and direction stay
/// with the row.
#[derive(Clone, Debug, Serialize)]
pub struct EventOut {
    pub row: String,
    pub wallet_label: Option<String>,
    pub amount: Dec,
    pub fee: Dec,
    pub spot_price: Fiat,
    pub fiat_value: Fiat,
    /// The row's text, else its id.
    pub description: String,
    pub at_regime: Option<Regime>,
    /// Why a disposal has its regime (`wahlrecht`).
    pub at_regime_basis: Option<&'static str>,
    pub cost_basis_pool_id: &'static str,
    pub at_swap_link: Option<String>,
}

/// A move the walk books (`NormalizedTaxTransfer`). Its legs' times, wallet
/// ids, and external id, and the pair's source, stay with the rows and the
/// pair.
#[derive(Clone, Debug, Serialize)]
pub struct TransferOut {
    /// The index into the request's `intra_pairs`.
    pub pair: usize,
    pub out: String,
    #[serde(rename = "in")]
    pub inbound: String,
    pub from_wallet_label: Option<String>,
    pub to_wallet_label: Option<String>,
    pub sent: Dec,
    pub received: Dec,
    pub fee: Dec,
    pub spot_price: Option<Fiat>,
    /// The out leg's text, else `Transfer <from> -> <to>`.
    pub description: String,
    pub from_cost_basis_pool_id: &'static str,
    pub to_cost_basis_pool_id: &'static str,
    pub at_regime: Option<Regime>,
    pub regime_flows: Option<RegimeFlows>,
    pub group_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_name_their_stage_and_class() {
        let unsupported =
            ErrorResponse::new(StagedError::contract(EngineError::unsupported("not yet")));
        assert_eq!(
            serde_json::to_value(&unsupported).unwrap(),
            serde_json::json!({
                "schema_version": 2,
                "ok": false,
                "error": {"stage": "unsupported", "inner": {"class": "Unsupported", "message": "not yet"}}
            })
        );
        let request = StagedError::contract(EngineError::request("bad"));
        assert_eq!(request.stage, Stage::Request);
        let mut gate = StagedError::new(Stage::ComputeMulti, EngineError::runtime("x"));
        gate.inner.class = InnerClass::AppError;
        gate.asset = Some(String::from("BTC"));
        assert_eq!(
            serde_json::to_value(&gate).unwrap(),
            serde_json::json!({
                "stage": "compute_multi",
                "asset": "BTC",
                "inner": {"class": "AppError", "message": "x"}
            })
        );
    }
}
