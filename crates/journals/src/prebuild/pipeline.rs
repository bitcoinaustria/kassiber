use super::*;
use serde_json::Value;

#[derive(Debug)]
/// Repaired inputs for the R4-13 repair/price progress-phase split. Always pass
/// this profile to pricing, loading, building and persistence.
pub struct RepairedInputs {
    pub profile: Value,
    pub source_overlap_repair: Option<Value>,
    pub source_overlap_warning: Option<Value>,
    pub consolidation: ConsolidationResult,
}

impl RepairedInputs {
    /// Optional fields appended after ordinary journal result fields, preserving
    /// Python dictionary order. `auto_priced` is an ordinary result field.
    pub fn result_fields(&self) -> Result<kassiber_pyjson::Value, Refusal> {
        let mut fields = source_overlap_result_fields(
            self.source_overlap_repair.as_ref(),
            self.source_overlap_warning.as_ref(),
        )?;
        if self.consolidation.activated != 0 {
            kassiber_envelope::set(
                &mut fields,
                "custody_pair_evidence_repairs",
                kassiber_pyjson::Value::Int(self.consolidation.activated.into()),
            );
        }
        Ok(fields)
    }
}

#[derive(Debug)]
pub struct Prepared {
    pub repaired: RepairedInputs,
    pub auto_priced: usize,
}

/// Source repair, warning, then legacy refresh/profile reload, atomically.
/// Requires the caller's reserved writer and whole-process savepoint.
pub fn repair_journal_inputs(
    conn: &Connection,
    profile: &Value,
    clock: &mut impl FnMut() -> Result<String, Refusal>,
    identity: &mut impl FnMut() -> Result<String, Refusal>,
) -> Result<RepairedInputs, Refusal> {
    require_writer(conn)?;
    atomic(conn, "r4_journal_repairs", || {
        let id = profile["id"]
            .as_str()
            .ok_or_else(|| Refusal::new("KeyError", "id"))?;
        let (repair, warning) = overlap::repair_and_warn(conn, id, clock, identity)?;
        let legacy = refresh_drifted_legacy_pair_components(conn, profile, clock)?;
        Ok(RepairedInputs {
            profile: legacy.profile,
            source_overlap_repair: repair,
            source_overlap_warning: warning,
            consolidation: legacy.consolidation,
        })
    })
}

/// Ordered repairs followed by cached/stored pricing, atomically. No provider
/// access, connection opening, implicit clock, entropy or transaction commit.
pub fn prepare_journals(
    conn: &Connection,
    profile: &Value,
    decimal_context: Context,
    clock: &mut impl FnMut() -> Result<String, Refusal>,
    identity: &mut impl FnMut() -> Result<String, Refusal>,
) -> Result<Prepared, Refusal> {
    require_writer(conn)?;
    atomic(conn, "r4_journal_prebuild", || {
        let repaired = repair_journal_inputs(conn, profile, clock, identity)?;
        let auto_priced = auto_price_transactions_from_rates_cache(
            conn,
            &PricingScope {
                profile_id: repaired.profile["id"]
                    .as_str()
                    .ok_or_else(|| Refusal::new("KeyError", "id"))?,
                fiat_currency: repaired.profile["fiat_currency"]
                    .as_str()
                    .ok_or_else(|| Refusal::new("KeyError", "fiat_currency"))?,
                decimal_context,
            },
        )?;
        Ok(Prepared {
            repaired,
            auto_priced,
        })
    })
}

/// Keep a savepoint across pre-build and the caller's build/persist continuation.
/// An error or unwind restores all these writes while retaining earlier caller
/// writes. The caller still owns final transaction commit/rollback. Callbacks
/// and external continuation side effects are not rewindable.
pub fn with_prebuild<T>(
    conn: &Connection,
    profile: &Value,
    decimal_context: Context,
    clock: &mut impl FnMut() -> Result<String, Refusal>,
    identity: &mut impl FnMut() -> Result<String, Refusal>,
    build_and_persist: impl FnOnce(&Prepared) -> Result<T, Refusal>,
) -> Result<(Prepared, T), Refusal> {
    require_writer(conn)?;
    atomic(conn, "r4_journal_pipeline", || {
        let prepared = prepare_journals(conn, profile, decimal_context, clock, identity)?;
        let result = build_and_persist(&prepared)?;
        Ok((prepared, result))
    })
}
