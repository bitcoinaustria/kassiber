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

#[test]
fn prebuild_sources_invalid_storage_refuses_before_json_projection() {
    let conn = Connection::open_in_memory().unwrap();
    let error = sources::rows(&conn, "SELECT CAST(X'80' AS TEXT) AS raw_json", []).unwrap_err();
    assert_eq!(error.family, "ValueError");
    assert_eq!(error.message, "invalid UTF-8");
    // The merged process stages keep native byte identity: BLOB evidence is
    // carried through the lossless codec rather than refused here.
    assert_eq!(
        sources::rows(&conn, "SELECT X'ff' AS raw_json", []).unwrap(),
        [serde_json::json!({"raw_json": {"$kassiber_bytes": "ff"}})]
    );
}

#[test]
fn prebuild_sqlite_refusals_retain_python_families() {
    for (code, family) in [
        (rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER, "IntegrityError"),
        (rusqlite::ffi::SQLITE_MISMATCH, "IntegrityError"),
        (rusqlite::ffi::SQLITE_TOOBIG, "DataError"),
        (rusqlite::ffi::SQLITE_NOMEM, "MemoryError"),
        (rusqlite::ffi::SQLITE_MISUSE, "ProgrammingError"),
        (rusqlite::ffi::SQLITE_CORRUPT, "DatabaseError"),
        (rusqlite::ffi::SQLITE_ERROR, "OperationalError"),
        (rusqlite::ffi::SQLITE_BUSY, "OperationalError"),
    ] {
        let refusal = Refusal::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            Some("original SQLite message".into()),
        ));
        assert_eq!(refusal.family, family);
        assert_eq!(refusal.message, "original SQLite message");
    }
}
