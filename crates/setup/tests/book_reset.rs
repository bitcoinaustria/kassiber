//! `core/book_reset.py`: kept reset records bind back by stable identity, or
//! hold the rows that came back. Byte parity with Python is pinned by the
//! frozen desktop vectors and `tests/test_rust_book_reset.py`; these cases pin
//! each rule on its own.
mod support;
use kassiber_envelope::get;
use kassiber_pyjson::Value;
use kassiber_setup::book_reset::{self, Settled};
use kassiber_store::rusqlite::Connection;
use support::*;

struct Book {
    f: Fixture,
    wid: String,
    pid: String,
}

fn book() -> Book {
    let f = Fixture::seed();
    let p = f.profile();
    let (wid, pid) = (text(&p, "workspace_id"), text(&p, "id"));
    f.book.connection().execute_batch(&format!(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('w','{wid}','{pid}','Cold','custom','{{}}','t');
         INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES
           ('out','{wid}','{pid}','w','ext-out','fp-out','2024-01-01T00:00:00Z','outbound','BTC',1000,10,'t'),
           ('in','{wid}','{pid}','w','ext-in','fp-in','2024-01-02T00:00:00Z','inbound','BTC',1000,0,'t'),
           ('loan','{wid}','{pid}','w','ext-loan','fp-loan','2024-02-01T00:00:00Z','outbound','BTC',500,0,'t');
         UPDATE transactions SET note='Kept', fiat_rate=51000.5, pricing_source_kind='manual_override' WHERE id='loan';
         INSERT INTO loan_legs(id,workspace_id,profile_id,transaction_id,role,created_at) VALUES('mark','{wid}','{pid}','loan','collateral','t');
         INSERT INTO transaction_pair_dismissals(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,reason,created_at) VALUES('dismissal','{wid}','{pid}','out','in','not a pair','t');
         INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,expected_leg_count,expected_allocation_count,created_at) VALUES('component','lineage','{wid}','{pid}',1,'transfer',2,0,'t');
         INSERT INTO custody_component_legs(id,component_id,workspace_id,profile_id,ordinal,role,rail,asset,exposure,conservation_unit,amount_msat,transaction_id,anchor_transaction_id,created_at) VALUES
           ('leg-out','component','{wid}','{pid}',0,'source','bitcoin','BTC','asset','BTC',1000,'out','out','t'),
           ('leg-in','component','{wid}','{pid}',1,'destination','bitcoin','BTC','asset','BTC',1000,'in','in','t');
         INSERT INTO custody_component_transaction_memberships(component_id,profile_id,transaction_id,created_at) VALUES('component','{pid}','out','t'),('component','{pid}','in','t');"
    ))
    .unwrap();
    Book { f, wid, pid }
}

impl Book {
    fn conn(&self) -> &Connection {
        self.f.book.connection()
    }
    fn reset(&self) -> Vec<(String, i64)> {
        let kept =
            book_reset::park_profile(self.conn(), &self.wid, &self.pid, "2026-01-01T00:00:00Z")
                .unwrap();
        self.conn()
            .execute("DELETE FROM transactions WHERE profile_id=?", [&self.pid])
            .unwrap();
        kept
    }
    /// The import path for one row: claim, insert under the claimed id, settle.
    fn reimport(
        &self,
        fingerprint: &str,
        external: &str,
        direction: &str,
        amount: i64,
    ) -> Option<Settled> {
        let id = book_reset::claim_transaction_id(
            self.conn(),
            &self.pid,
            "w",
            fingerprint,
            Some(external),
            "BTC",
            direction,
        )
        .unwrap()?;
        self.conn().execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES(?1,?2,?3,'w',?4,?5,'2024-01-01T00:00:00Z',?6,'BTC',?7,?8,'t2')",
            kassiber_store::rusqlite::params![id, self.wid, self.pid, external, fingerprint, direction, amount, if external == "ext-out" { 10 } else { 0 }],
        )
        .unwrap();
        Some(book_reset::settle_claim(self.conn(), &id).unwrap())
    }
    fn count(&self, sql: &str) -> i64 {
        self.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn holds(&self) -> Vec<(String, String)> {
        book_reset::held_quarantines(self.conn(), &self.pid)
            .unwrap()
            .iter()
            .map(|q| (text(q, "transaction_id"), text(q, "detail_json")))
            .collect()
    }
}

fn counts(b: &Book) -> (i64, i64) {
    let c = book_reset::counts(b.conn(), &b.pid).unwrap();
    let n = |k| match get(&c, k) {
        Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap(),
        _ => panic!(),
    };
    (n("held"), n("waiting"))
}

#[test]
fn reset_parks_decisions_and_records_identities() {
    let b = book();
    let kept = b.reset();
    let get = |t: &str| kept.iter().find(|(k, _)| k == t).unwrap().1;
    assert_eq!(get("loan_legs"), 1);
    assert_eq!(get("transaction_pair_dismissals"), 1);
    assert_eq!(get("custody_component_transaction_memberships"), 2);
    assert_eq!(get("transaction_metadata"), 1);
    assert_eq!(kept.last().unwrap().0, "transaction_metadata");
    assert_eq!(b.count("SELECT count(*) FROM book_reset_transactions"), 3);
    assert_eq!(b.count("SELECT count(*) FROM loan_legs"), 0);
    // Legs keep their anchor; the delete only nulls the transaction.
    assert_eq!(b.count("SELECT count(*) FROM custody_component_legs WHERE transaction_id IS NULL AND anchor_transaction_id IS NOT NULL"), 2);
    let metadata: String = b
        .conn()
        .query_row(
            "SELECT metadata_json FROM book_reset_transactions WHERE transaction_id='loan'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        metadata.starts_with(
            r#"{"fiat_currency": null, "fiat_price_source": null, "fiat_rate": 51000.5"#
        ),
        "{metadata}"
    );
    assert!(metadata.contains(r#""note": "Kept""#));
    // A component's memberships wait for every leg of the component.
    let ids: Vec<String> = b.conn().prepare("SELECT transaction_ids_json FROM book_reset_parked_rows WHERE table_name='custody_component_transaction_memberships' ORDER BY seq").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    assert_eq!(ids, [r#"["out", "in"]"#, r#"["out", "in"]"#]);
}

#[test]
fn claims_prefer_the_exact_fingerprint_then_one_identity() {
    let b = book();
    b.reset();
    // A changed fingerprint still claims the only matching identity.
    assert_eq!(
        b.reimport("fp-other", "ext-in", "inbound", 1000),
        Some(Settled::Bound)
    );
    // Another direction is another identity.
    assert_eq!(b.reimport("fp-x", "ext-out", "inbound", 1000), None);
    // Exact fingerprint first.
    assert_eq!(
        b.reimport("fp-out", "ext-out", "outbound", 1000),
        Some(Settled::Bound)
    );
    // A claimed identity is not claimed twice.
    assert_eq!(b.reimport("fp-out-2", "ext-out", "outbound", 1000), None);
}

#[test]
fn an_ambiguous_identity_claims_nothing() {
    let b = book();
    b.conn().execute_batch(&format!(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('in-2','{}','{}','w','ext-in','fp-in-2','2024-01-03T00:00:00Z','inbound','BTC',7,0,'t')",
        b.wid, b.pid
    )).unwrap();
    b.reset();
    assert_eq!(b.reimport("fp-other", "ext-in", "inbound", 1000), None);
    assert_eq!(
        b.reimport("fp-in", "ext-in", "inbound", 1000),
        Some(Settled::Bound)
    );
}

#[test]
fn settling_binds_metadata_and_legs_or_marks_changed_facts() {
    let b = book();
    b.reset();
    assert_eq!(
        b.reimport("fp-loan", "ext-loan", "outbound", 500),
        Some(Settled::BoundWithMetadata)
    );
    let (note, rate, kind): (String, f64, String) = b
        .conn()
        .query_row(
            "SELECT note,fiat_rate,pricing_source_kind FROM transactions WHERE id='loan'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (note.as_str(), rate, kind.as_str()),
        ("Kept", 51000.5, "manual_override")
    );
    assert_eq!(
        b.count("SELECT count(*) FROM book_reset_transactions WHERE transaction_id='loan'"),
        0
    );
    // Different facts keep the old id but bind nothing.
    assert_eq!(
        b.reimport("fp-new", "ext-out", "outbound", 999),
        Some(Settled::Changed)
    );
    let (state, fields): (String, String) = b.conn().query_row("SELECT state,changed_fields_json FROM book_reset_transactions WHERE transaction_id='out'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(
        (state.as_str(), fields.as_str()),
        ("changed", r#"["amount"]"#)
    );
    assert_eq!(b.count("SELECT count(*) FROM custody_component_legs WHERE id='leg-out' AND transaction_id IS NULL"), 1);
    // A bound row gets its leg back.
    b.reimport("fp-in", "ext-in", "inbound", 1000);
    assert_eq!(
        b.count(
            "SELECT count(*) FROM custody_component_legs WHERE id='leg-in' AND transaction_id='in'"
        ),
        1
    );
}

#[test]
fn parked_rows_bind_only_when_every_transaction_is_bound() {
    let b = book();
    b.reset();
    b.reimport("fp-out", "ext-out", "outbound", 1000);
    assert_eq!(book_reset::rebind_parked(b.conn(), &b.pid).unwrap(), 0);
    assert_eq!(counts(&b), (3, 2));
    b.reimport("fp-in", "ext-in", "inbound", 1000);
    // The dismissal and both memberships, in parent-first order.
    assert_eq!(book_reset::rebind_parked(b.conn(), &b.pid).unwrap(), 3);
    assert_eq!(
        b.count("SELECT count(*) FROM custody_component_transaction_memberships"),
        2
    );
    assert_eq!(
        b.count("SELECT count(*) FROM transaction_pair_dismissals"),
        1
    );
    assert_eq!(counts(&b), (0, 2));
    b.reimport("fp-loan", "ext-loan", "outbound", 500);
    assert_eq!(book_reset::rebind_parked(b.conn(), &b.pid).unwrap(), 1);
    assert_eq!(counts(&b), (0, 0));
    assert!(!book_reset::has_state(b.conn(), &b.pid).unwrap());
    assert_eq!(
        b.count("SELECT count(*) FROM loan_legs WHERE transaction_id='loan'"),
        1
    );
}

#[test]
fn a_taken_place_keeps_the_decision_parked() {
    let b = book();
    b.reset();
    b.reimport("fp-loan", "ext-loan", "outbound", 500);
    // A newer mark on the same transaction occupies the active slot.
    b.conn().execute(&format!("INSERT INTO loan_legs(id,workspace_id,profile_id,transaction_id,role,created_at) VALUES('newer','{}','{}','loan','collateral','t3')", b.wid, b.pid), []).unwrap();
    assert_eq!(book_reset::rebind_parked(b.conn(), &b.pid).unwrap(), 0);
    assert_eq!(
        b.count("SELECT count(*) FROM book_reset_parked_rows WHERE table_name='loan_legs'"),
        1
    );
    assert_eq!(b.holds().len(), 1);
}

#[test]
fn holds_name_the_missing_and_changed_transactions() {
    let b = book();
    b.reset();
    assert!(b.holds().is_empty(), "nothing has come back yet");
    b.reimport("fp-new", "ext-out", "outbound", 999);
    b.reimport("fp-loan-new", "ext-loan", "outbound", 400);
    // A changed transaction binds nothing, even with every row present.
    assert_eq!(book_reset::rebind_parked(b.conn(), &b.pid).unwrap(), 0);
    let holds = b.holds();
    let ids: Vec<&str> = holds.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["loan", "out"]);
    assert_eq!(
        holds[0].1,
        r#"{"changed_transactions": [{"fields": ["amount"], "transaction_id": "loan"}], "held_decisions": {"loan_legs": 1}, "metadata_held": true, "missing_transactions": []}"#
    );
    assert_eq!(
        holds[1].1,
        r#"{"changed_transactions": [{"fields": ["amount"], "transaction_id": "out"}], "held_decisions": {"custody_component_transaction_memberships": 2, "transaction_pair_dismissals": 1}, "metadata_held": false, "missing_transactions": [{"asset": "BTC", "direction": "inbound", "external_id": "ext-in", "transaction_id": "in", "wallet_id": "w"}]}"#
    );
    let q = &book_reset::held_quarantines(b.conn(), &b.pid).unwrap()[0];
    assert_eq!(text(q, "reason"), "book_reset_decision_held");
    assert_eq!(text(q, "workspace_id"), b.wid);
    assert_eq!(counts(&b), (5, 0));
}

#[test]
fn the_wipe_discards_every_kept_record() {
    let b = book();
    b.reset();
    let discarded = book_reset::discard_profile(b.conn(), &b.pid).unwrap();
    assert_eq!(
        discarded,
        [
            ("book_reset_transactions".to_owned(), 3),
            ("book_reset_parked_rows".to_owned(), 4)
        ]
    );
    assert!(!book_reset::has_state(b.conn(), &b.pid).unwrap());
}

#[test]
fn schemas_without_the_tables_have_no_state() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE book_reset_transactions(x)")
        .unwrap();
    assert!(!book_reset::has_state(&conn, "p").unwrap());
    assert!(book_reset::held_quarantines(&conn, "p").unwrap().is_empty());
    assert!(book_reset::parked_attachment_relpaths(&conn)
        .unwrap()
        .is_empty());
}
