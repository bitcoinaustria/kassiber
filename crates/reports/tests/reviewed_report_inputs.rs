//! Country reports read the stored journal the way the review figures do:
//! a transaction reviewed as not taxable is on no form, and amounts round
//! from the exact decimals, not their REAL copies.
use kassiber_envelope::{dict, get};
use kassiber_pyjson::Value;
use kassiber_store::Book;

const FIXTURE: &str = include_str!("data/book.sql");

#[test]
fn us_form_8949_respects_reviewed_taxability() {
    let directory = tempfile::tempdir().unwrap();
    let book = Book::create(directory.path(), None).unwrap();
    let conn = book.connection();
    conn.execute_batch(FIXTURE).unwrap();
    conn.execute_batch(
        "DELETE FROM journal_quarantines;
         DELETE FROM journal_entries;
         UPDATE profiles SET tax_country='us', fiat_currency='USD' WHERE id='p';
         UPDATE transactions SET taxability_override=0 WHERE id='no-tax';
         INSERT INTO journal_entries
             (id,workspace_id,profile_id,transaction_id,wallet_id,account_id,occurred_at,
              entry_type,asset,quantity,proceeds,cost_basis,gain_loss,capital_gains_type,created_at)
         VALUES ('hidden','ws','p','no-tax','hot','acct','2025-02-01T00:00:00Z',
                 'disposal','BTC',-100001,100,10,90,'short','fixed');",
    )
    .unwrap();
    let (_, profile) = kassiber_setup::references::scope(conn, &dict::<&str>([])).unwrap();
    let cut = kassiber_reports::tax_years::Cut::of(conn, &profile).unwrap();
    let review = kassiber_reports::review_figures::year_totals(conn, "p", cut).unwrap();
    assert!(review.is_empty());
    let form = kassiber_reports::us_tax::model(conn, &profile, None).unwrap();
    assert_eq!(get(&form, "rows"), Some(&Value::List(vec![])));
}

#[test]
fn austrian_kz_174_rounds_the_exact_stored_gain() {
    let directory = tempfile::tempdir().unwrap();
    let book = Book::create(directory.path(), None).unwrap();
    let conn = book.connection();
    conn.execute_batch(FIXTURE).unwrap();
    conn.execute_batch(
        "DELETE FROM journal_quarantines;
         DELETE FROM journal_entries;
         UPDATE profiles SET tax_country='at', fiat_currency='EUR' WHERE id='p';
         INSERT INTO journal_entries
             (id,workspace_id,profile_id,transaction_id,wallet_id,account_id,occurred_at,
              entry_type,asset,quantity,proceeds,proceeds_exact,cost_basis,cost_basis_exact,
              gain_loss,gain_loss_exact,at_category,created_at)
         VALUES ('fraction','ws','p','out','cold','acct','2025-02-01T00:00:00Z',
                 'disposal','BTC',-1000,1.005,'1.00499999999999999',0,'0',
                 1.005,'1.00499999999999999','neu_gain','fixed');",
    )
    .unwrap();
    let (_, profile) = kassiber_setup::references::scope(conn, &dict::<&str>([])).unwrap();
    let (kennzahlen, _) =
        kassiber_reports::review_figures::kennzahl_summary(conn, &profile, 2025).unwrap();
    let (_, cents) = kennzahlen.into_iter().find(|row| row.0 == 174).unwrap();
    assert_eq!(cents, 100.into());
}
