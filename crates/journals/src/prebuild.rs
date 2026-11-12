use crate::Refusal;
use kassiber_pydecimal::{Context, Decimal};
use kassiber_store::rusqlite::{
    self, functions::FunctionFlags, params, types::Value as Sql, Connection,
};

mod legacy;
mod overlap;
mod pipeline;
mod pricing;
mod sources;

pub use legacy::{refresh_drifted_legacy_pair_components, ConsolidationResult, LegacyRefresh};
pub use overlap::{
    repair_journal_source_overlaps, source_overlap_blocker, source_overlap_result_fields,
    source_overlap_warning,
};
pub use pipeline::{
    prepare_journals, repair_journal_inputs, with_prebuild, Prepared, RepairedInputs,
};

pub struct PricingScope<'a> {
    pub profile_id: &'a str,
    pub fiat_currency: &'a str,
    pub decimal_context: Context,
}

pub fn auto_price_transactions_from_rates_cache(
    conn: &Connection,
    scope: &PricingScope<'_>,
) -> Result<usize, Refusal> {
    require_writer(conn)?;
    atomic(conn, "r4_journal_pricing", || pricing::run(conn, scope))
}

fn require_writer(conn: &Connection) -> Result<(), Refusal> {
    if conn.transaction_state(Some("main"))? != rusqlite::TransactionState::Write {
        return Err(Refusal::new(
            "RuntimeError",
            "journal pre-build requires the caller's write transaction",
        ));
    }
    Ok(())
}

fn atomic<T>(
    conn: &Connection,
    name: &'static str,
    f: impl FnOnce() -> Result<T, Refusal>,
) -> Result<T, Refusal> {
    kassiber_store::begin_write_savepoint(conn, name)?;
    let mut savepoint = Savepoint {
        conn: Some(conn),
        name,
    };
    let result = f()?;
    conn.execute_batch(&format!("RELEASE {name}"))?;
    savepoint.conn = None;
    Ok(result)
}

struct Savepoint<'a> {
    conn: Option<&'a Connection>,
    name: &'static str,
}
impl Drop for Savepoint<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn {
            let name = self.name;
            let _ = conn.execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}"));
        }
    }
}

pub(crate) fn app(error: kassiber_envelope::AppError) -> Refusal {
    let mut refusal = Refusal::new("AppError", error.message);
    refusal.code = Some(error.code);
    refusal.hint = error.hint.map(String::from);
    refusal.retryable = Some(error.retryable);
    refusal.details = error
        .details
        .as_ref()
        .map(kassiber_custody::codec::to_json)
        .unwrap_or_default();
    refusal.ordered_details = error.details;
    refusal
}

#[cfg(test)]
mod tests;
