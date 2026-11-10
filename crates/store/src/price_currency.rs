//! A stored price counts only in the currency it was stored in
//! (`kassiber/core/pricing.py`, `price_currency_mismatch`).
//!
//! A book's currency can change after its rows were priced. Journals hold a
//! priced row whose currency is not the book's (`price_currency_mismatch`);
//! readers that take stored prices as the book's (fallback valuations,
//! flows) read such a price as no price, with this predicate.

/// The quarantine reason of a price stored in another currency.
pub const REASON: &str = "price_currency_mismatch";

/// A stored currency as journals compare it: SQLite's `UPPER(TRIM(x))`,
/// spaces stripped and ASCII letters upper-cased.
pub fn currency_code(value: &str) -> String {
    value.trim_matches(' ').to_ascii_uppercase()
}

/// `pricing.price_in_book_currency_sql`: the transaction's price, if any,
/// counts in its book's currency. `alias` names the transactions table.
pub fn price_in_book_currency_sql(alias: &str) -> String {
    let column = format!("{alias}.fiat_currency");
    format!(
        "(TRIM(COALESCE({column}, '')) = '' OR UPPER(TRIM({column})) = \
         (SELECT UPPER(TRIM(COALESCE(book.fiat_currency, ''))) FROM profiles book \
         WHERE book.id = {alias}.profile_id))"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn the_predicate_is_pythons_text_and_compares_like_currency_code() {
        assert_eq!(
            price_in_book_currency_sql("t"),
            "(TRIM(COALESCE(t.fiat_currency, '')) = '' OR UPPER(TRIM(t.fiat_currency)) = \
             (SELECT UPPER(TRIM(COALESCE(book.fiat_currency, ''))) FROM profiles book \
             WHERE book.id = t.profile_id))"
        );
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE profiles(id TEXT, fiat_currency TEXT);
             CREATE TABLE transactions(id TEXT, profile_id TEXT, fiat_currency TEXT);
             INSERT INTO profiles VALUES('p', ' usd ');",
        )
        .unwrap();
        for (currency, counts) in [
            (None, true),
            (Some(""), true),
            (Some("  "), true),
            (Some("USD"), true),
            (Some(" usd"), true),
            (Some("EUR"), false),
            (Some("\tUSD"), false),
        ] {
            conn.execute("DELETE FROM transactions", []).unwrap();
            conn.execute("INSERT INTO transactions VALUES('t','p',?)", [currency])
                .unwrap();
            let sql = format!(
                "SELECT {} FROM transactions",
                price_in_book_currency_sql("transactions")
            );
            let found: bool = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
            assert_eq!(found, counts, "{currency:?}");
            let code = currency_code(currency.unwrap_or(""));
            assert_eq!(code.is_empty() || code == currency_code(" usd "), counts);
        }
    }
}
