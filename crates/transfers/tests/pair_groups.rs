#![cfg(feature = "test-harness")]
//! A one-to-many manual link authored in one review apply must slice the same
//! way in the plan and in the apply. Its pairs share `created_at` and draw
//! fresh ids, so a grouping ordered by id would hand the send's network fee to
//! whichever receipt drew the smallest id, and the apply's journal entries
//! would disagree with the previewed ones (`review_plan_stale`).
use kassiber_envelope::{dict, s, AppError};
use kassiber_journals::review_workflow::{self, Error};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};
use kassiber_transfers::{Scope, Sources};
use std::sync::atomic::{AtomicI64, Ordering};

const CLOCK: &str = "2026-10-31T12:00:00Z";

/// Ids drawn in ascending or descending order, so the pairs one apply authors
/// sort by id in operation order or in reverse.
struct Draws {
    next: AtomicI64,
    step: i64,
}
impl Draws {
    fn ascending() -> Self {
        Self {
            next: AtomicI64::new(1),
            step: 1,
        }
    }
    fn descending() -> Self {
        Self {
            next: AtomicI64::new(0xffff_ffff_fff0),
            step: -1,
        }
    }
}
impl Sources for Draws {
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        let n = self.next.fetch_add(self.step, Ordering::Relaxed);
        Ok(format!("aaaaaaaa-aaaa-4aaa-8aaa-{n:012x}"))
    }
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        Ok(CLOCK.into())
    }
}

fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}

/// A book with a priced purchase, one send carrying a network fee, and three
/// receipts in another owned wallet that together take the send's principal.
fn book() -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    book.connection()
        .execute_batch(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z');
             INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','generic','2025-01-01T00:00:00Z');
             INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('source','w','p','source','custom','2025-01-01T00:00:00Z');
             INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('target','w','p','target','custom','2025-01-01T00:00:00Z');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,fiat_currency,fiat_rate,fiat_value_exact,raw_json,created_at)
               VALUES('buy','w','p','source','fp-buy','buy-ext','BTC','inbound',100000000000,0,0,'2025-01-01T00:00:00Z','buy','EUR','50000','50000','{}','2025-01-01T00:00:00Z');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,fiat_currency,fiat_rate,fiat_value_exact,raw_json,created_at)
               VALUES('send','w','p','source','fp-send','send-ext','BTC','outbound',6000000000,2000000,0,'2025-02-01T00:00:00Z','withdrawal','EUR','60000','3600','{}','2025-01-01T00:00:00Z');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,fiat_currency,fiat_rate,fiat_value_exact,raw_json,created_at)
               VALUES('receipt-1','w','p','target','fp-r1','r1-ext','BTC','inbound',1000000000,0,0,'2025-02-01T00:00:00Z','deposit','EUR','60000','600','{}','2025-01-01T00:00:00Z');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,fiat_currency,fiat_rate,fiat_value_exact,raw_json,created_at)
               VALUES('receipt-2','w','p','target','fp-r2','r2-ext','BTC','inbound',2000000000,0,0,'2025-02-01T00:00:00Z','deposit','EUR','60000','1200','{}','2025-01-01T00:00:00Z');
             INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,fiat_currency,fiat_rate,fiat_value_exact,raw_json,created_at)
               VALUES('receipt-3','w','p','target','fp-r3','r3-ext','BTC','inbound',3000000000,0,0,'2025-02-01T00:00:00Z','deposit','EUR','60000','1800','{}','2025-01-01T00:00:00Z');",
        )
        .unwrap();
    (dir, book)
}

fn operations() -> Value {
    Value::List(
        ["receipt-1", "receipt-2", "receipt-3"]
            .into_iter()
            .map(|receipt| {
                dict([
                    ("type", s("pair")),
                    ("out_transaction_id", s("send")),
                    ("in_transaction_id", s(receipt)),
                    ("pair_kind", s("manual")),
                    ("policy", s("carrying-value")),
                    ("reason", s("One send split across three receipts")),
                ])
            })
            .collect(),
    )
}

fn input_version(conn: &Connection) -> Value {
    conn.query_row(
        "SELECT journal_input_version FROM profiles WHERE id='p'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .unwrap()
    .into()
}

/// Journal entries as the economic rows they book, without their identities.
fn entries(conn: &Connection) -> Vec<(String, String, String, String)> {
    conn.prepare(
        "SELECT COALESCE(transaction_id,''),entry_type,CAST(quantity AS TEXT),wallet_id
         FROM journal_entries WHERE profile_id='p' ORDER BY 1,2,3,4",
    )
    .unwrap()
    .query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// The receipt whose slice carries the send's network fee: the destination fed
/// by the source leg that also feeds the fee leg.
fn fee_slice(conn: &Connection) -> String {
    let slices = conn
        .prepare(
            "SELECT destination.transaction_id
             FROM custody_components component
             JOIN custody_component_legs source ON source.component_id=component.id AND source.role='source'
             JOIN custody_component_allocations to_fee ON to_fee.source_leg_id=source.id
             JOIN custody_component_legs fee ON fee.id=to_fee.sink_leg_id AND fee.role='fee'
             JOIN custody_component_allocations onward ON onward.source_leg_id=source.id
             JOIN custody_component_legs destination ON destination.id=onward.sink_leg_id AND destination.role='destination'
             WHERE component.profile_id='p' AND component.state='active'",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(slices.len(), 1, "one slice carries the fee: {slices:?}");
    slices.into_iter().next().unwrap()
}

/// Plan with ids drawn one way, apply with ids drawn the other way.
fn plan_then_apply(plan: &Draws, apply: &Draws) -> Result<(Book, tempfile::TempDir), Error> {
    let (dir, book) = book();
    let conn = book.connection();
    let artifact = review_workflow::execute(
        conn,
        &scope(),
        "review.plan",
        &dict([
            ("operations", operations()),
            ("expected_input_version", input_version(conn)),
        ]),
        "user",
        plan,
    )?;
    review_workflow::execute(
        conn,
        &scope(),
        "review.apply",
        &dict([
            ("artifact", artifact),
            ("idempotency_key", s("one-to-many")),
        ]),
        "user",
        apply,
    )?;
    Ok((book, dir))
}

#[test]
fn a_one_to_many_manual_link_applies_as_planned_whatever_ids_its_pairs_draw() {
    let mut booked = None;
    for (plan, apply) in [
        (Draws::ascending(), Draws::ascending()),
        (Draws::ascending(), Draws::descending()),
        (Draws::descending(), Draws::ascending()),
        (Draws::descending(), Draws::descending()),
    ] {
        let (book, _dir) = match plan_then_apply(&plan, &apply) {
            Ok(applied) => applied,
            Err(Error::Service(error)) => panic!("apply refused: {} {}", error.code, error.message),
            Err(Error::Build(error)) => panic!("apply failed: {}", error.message),
        };
        let conn = book.connection();
        let result = (entries(conn), fee_slice(conn));
        assert!(!result.0.is_empty(), "the apply booked journal entries");
        // The pairs sort by their transactions, so the first receipt's slice
        // carries the fee whatever ids they drew.
        assert_eq!(result.1, "receipt-1");
        match &booked {
            None => booked = Some(result),
            Some(first) => assert_eq!(&result, first, "the same link books the same way"),
        }
    }
}
