use super::*;
use kassiber_pydecimal::Rounding;

fn scope() -> PricingScope<'static> {
    PricingScope {
        profile_id: "p",
        fiat_currency: "EUR",
        decimal_context: Context::new(32, Rounding::HalfEven),
    }
}

#[test]
fn prebuild_pricing_requires_caller_write_transaction() {
    let conn = Connection::open_in_memory().unwrap();
    assert_eq!(
        auto_price_transactions_from_rates_cache(&conn, &scope())
            .unwrap_err()
            .message,
        "journal pre-build requires the caller's write transaction"
    );
    conn.execute_batch("BEGIN").unwrap();
    assert!(auto_price_transactions_from_rates_cache(&conn, &scope()).is_err());
    conn.execute_batch("ROLLBACK").unwrap();
}
