//! `create_evidenced` pairs only what the provider's evidence fully
//! reconciles: one asset, an outbound debit that is exactly the inbound
//! amount plus the stated (non-negative) fee. Anything else is refused, never
//! activated as an exact, basis-carrying transfer.
use kassiber_envelope::AppError;
use kassiber_store::{rusqlite::Connection, Book};
use kassiber_transfers::{create_evidenced, Evidenced, Scope, Sources};

struct Fixed;
impl Sources for Fixed {
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        Ok("aaaaaaaa-aaaa-4aaa-8aaa-000000000001".into())
    }
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        Ok("2026-11-19T12:00:00Z".into())
    }
}

fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}

/// A send of `out_msat` plus `fee_msat` from one owned wallet and a receipt
/// of `in_msat` of `in_asset` in another.
fn book(out_msat: i64, fee_msat: i64, in_msat: i64, in_asset: &str) -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    book.connection()
        .execute_batch(&format!(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z');
             INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','generic','2025-01-01T00:00:00Z');
             INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('source','w','p','source','custom','2025-01-01T00:00:00Z');
             INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('target','w','p','target','custom','2025-01-01T00:00:00Z');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at)
               VALUES('send','w','p','source','fp-send','send-ext','BTC','outbound',{out_msat},{fee_msat},0,'2025-02-01T00:00:00Z','withdrawal','{{}}','2025-01-01T00:00:00Z');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at)
               VALUES('receipt','w','p','target','fp-receipt','receipt-ext','{in_asset}','inbound',{in_msat},0,0,'2025-02-01T00:00:01Z','deposit','{{}}','2025-01-01T00:00:00Z');"
        ))
        .unwrap();
    (dir, book)
}

fn evidenced(book: &Book, fee: i128) -> Result<Evidenced, AppError> {
    create_evidenced(
        book.connection(),
        &scope(),
        "send",
        "receipt",
        fee,
        "provider export names the txid",
        &Fixed,
    )
}

fn components(book: &Book) -> i64 {
    book.connection()
        .query_row("SELECT count(*) FROM custody_components", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_reconciled_same_asset_pair_with_its_stated_fee_is_created() {
    let (_dir, book) = book(1_000_000, 2_000, 1_000_000, "BTC");
    assert!(matches!(evidenced(&book, 2_000), Ok(Evidenced::Created(_))));
    assert_eq!(components(&book), 1);
}

#[test]
fn a_negative_stated_fee_is_refused() {
    let (_dir, book) = book(1_000_000, 0, 1_000_000, "BTC");
    let error = evidenced(&book, -5).unwrap_err();
    assert_eq!(error.code, "validation", "{error:?}");
    assert_eq!(components(&book), 0);
}

#[test]
fn an_inbound_amount_the_debit_does_not_cover_is_refused() {
    let (_dir, book) = book(100_000, 0, 110_000, "BTC");
    let error = evidenced(&book, 0).unwrap_err();
    assert_eq!(error.code, "validation", "{error:?}");
    assert_eq!(components(&book), 0);
}

#[test]
fn legs_of_different_assets_are_refused() {
    let (_dir, book) = book(1_000_000, 0, 1_000_000, "LBTC");
    let error = evidenced(&book, 0).unwrap_err();
    assert_eq!(error.code, "validation", "{error:?}");
    assert_eq!(components(&book), 0);
}
