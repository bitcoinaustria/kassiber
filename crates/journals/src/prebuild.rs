use crate::Refusal;
use kassiber_pydecimal::{Context, Decimal};
use kassiber_store::rusqlite::{
    self, functions::FunctionFlags, params, types::Value as Sql, Connection,
};

mod pricing;

pub struct PricingScope<'a> {
    pub profile_id: &'a str,
    pub fiat_currency: &'a str,
    pub decimal_context: Context,
}

pub fn auto_price_transactions_from_rates_cache(
    conn: &Connection,
    scope: &PricingScope<'_>,
) -> Result<usize, Refusal> {
    if conn.transaction_state(Some("main"))? != rusqlite::TransactionState::Write {
        return Err(Refusal::new(
            "RuntimeError",
            "journal pre-build requires the caller's write transaction",
        ));
    }
    conn.execute_batch("SAVEPOINT r4_journal_pricing")?;
    let mut savepoint = Savepoint(Some(conn));
    let count = pricing::run(conn, scope)?;
    conn.execute_batch("RELEASE r4_journal_pricing")?;
    savepoint.0 = None;
    Ok(count)
}

struct Savepoint<'a>(Option<&'a Connection>);
impl Drop for Savepoint<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.0 {
            let _ =
                conn.execute_batch("ROLLBACK TO r4_journal_pricing; RELEASE r4_journal_pricing");
        }
    }
}

fn app(error: kassiber_envelope::AppError) -> Refusal {
    let mut refusal = Refusal::new("AppError", error.message);
    refusal.code = Some(error.code);
    refusal.hint = error.hint;
    refusal.retryable = Some(error.retryable);
    refusal.details = error
        .details
        .as_ref()
        .map(kassiber_custody::codec::to_json)
        .unwrap_or_default();
    refusal
}

#[cfg(test)]
mod tests;
