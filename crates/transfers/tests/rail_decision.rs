//! A country whose registry entry requires a rail decision (Germany,
//! DE-RAIL; the United Kingdom, its Bitcoin rails rule) books a BTC/LBTC
//! rail change only with the pair's own policy:
//! the book's `bitcoin_rail_carrying_value` default does not decide it.
use kassiber_envelope::{dict, get, py_str, s};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};
use kassiber_transfers::{execute_with_sources, Scope, Sources};

struct Fixed;
impl Sources for Fixed {
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("00000000-0000-4000-8000-000000000001".into())
    }
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2025-06-01T00:00:00Z".into())
    }
}

fn book(country: &str) -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    c.execute_batch(&format!(
        "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z');
         INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,bitcoin_rail_carrying_value,created_at)
           VALUES('p','w','Book','EUR','{country}',1,'2025-01-01T00:00:00Z');
         INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('btc','w','p','btc','custom','2025-01-01T00:00:00Z');
         INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('liquid','w','p','liquid','custom','2025-01-01T00:00:00Z');
         INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at)
           VALUES('out','w','p','btc','fp-out','out-ext','BTC','outbound',5000000000,0,0,'2024-02-10T09:00:00Z','swap','{{}}','2025-01-01T00:00:00Z');
         INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at)
           VALUES('in','w','p','liquid','fp-in','in-ext','LBTC','inbound',4999000000,0,0,'2024-02-10T09:00:00Z','swap','{{}}','2025-01-01T00:00:00Z');"
    ))
    .unwrap();
    (dir, book)
}

fn pair(c: &Connection, policy: Option<&str>) -> Result<Value, kassiber_envelope::AppError> {
    let mut args = vec![("tx_out", s("out")), ("tx_in", s("in"))];
    if let Some(policy) = policy {
        args.push(("policy", s(policy)));
    }
    execute_with_sources(
        c,
        &Scope {
            workspace_id: "w",
            profile_id: "p",
        },
        "transfers.pair",
        &dict(args),
        &Fixed,
    )
}

fn stored_policy(c: &Connection) -> String {
    c.query_row(
        "SELECT tax_policy FROM custody_component_economic_terms",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn rail_pairs_need_an_explicit_policy_where_the_country_requires_one() {
    // de-24: the book's default (carrying value) does not decide the pair.
    let (_dir, de) = book("de");
    let error = pair(de.connection(), None).unwrap_err();
    assert_eq!(error.code, "rail_policy_required");
    assert!(error.message.contains("BTC/LBTC"), "{}", error.message);
    let count: i64 = de
        .connection()
        .query_row("SELECT COUNT(*) FROM custody_components", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "nothing is written");
    // An explicit policy decides it, either way.
    let result = pair(de.connection(), Some("taxable")).unwrap();
    assert_eq!(
        get(&result, "policy").map(py_str).as_deref(),
        Some("taxable")
    );
    assert_eq!(stored_policy(de.connection()), "taxable");
    let (_dir, de) = book("de");
    pair(de.connection(), Some("carrying-value")).unwrap();
    assert_eq!(stored_policy(de.connection()), "carrying-value");
    // A generic book keeps its default.
    let (_dir, generic) = book("generic");
    let result = pair(generic.connection(), None).unwrap();
    assert_eq!(
        get(&result, "policy").map(py_str).as_deref(),
        Some("carrying-value")
    );
}

#[test]
fn united_kingdom_rail_pairs_also_need_their_own_policy() {
    // tax-country-gb.md, Bitcoin rails: a peg is held without the book's
    // decision, exactly as Germany's is.
    let (_dir, gb) = book("gb");
    let error = pair(gb.connection(), None).unwrap_err();
    assert_eq!(error.code, "rail_policy_required");
    let count: i64 = gb
        .connection()
        .query_row("SELECT COUNT(*) FROM custody_components", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "nothing is written");
    pair(gb.connection(), Some("taxable")).unwrap();
    assert_eq!(stored_policy(gb.connection()), "taxable");
    // Ireland's treatment is open (IE-Q9): its book keeps the default.
    let (_dir, ie) = book("ie");
    let result = pair(ie.connection(), None).unwrap();
    assert_eq!(
        get(&result, "policy").map(py_str).as_deref(),
        Some("carrying-value")
    );
}
