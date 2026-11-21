//! "Wipe everything" discards the opt-in bank-statement tables with the
//! rest of the book's data (docs/reference/bank-statements.md), through the
//! real reset path; "keep decisions" leaves them alone.
mod support;
use kassiber_envelope::{dict, get};
use kassiber_pyjson::Value;
use kassiber_setup::profile_reset;
use kassiber_store::rusqlite::Connection;
use support::*;

fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn seed_bank(f: &Fixture) -> (String, String) {
    let conn = f.book.connection();
    let (wid, pid): (String, String) = conn
        .query_row("SELECT workspace_id,id FROM profiles", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    kassiber_store::optional::ensure_bank_statement_schema(conn).unwrap();
    conn.execute_batch("COMMIT").unwrap();
    let sql = "INSERT INTO external_documents(id,workspace_id,profile_id,document_type,label,created_at,updated_at) VALUES('doc','W','P','statement','Statement','2025','2025');
INSERT INTO attachments(id,workspace_id,profile_id,attachment_type,label,created_at) VALUES('file','W','P','url','statement','2025');
INSERT INTO external_document_attachments(document_id,attachment_id,created_at) VALUES('doc','file','2025');
INSERT INTO bank_statement_imports(id,workspace_id,profile_id,document_id,attachment_id,source_format,file_sha256,file_key,account_keys,imported_at) VALUES('imp','W','P','doc','file','csv','sha-of-the-file','key-of-the-file','accounts-of-the-file','2025');
INSERT INTO bank_statement_lines(id,workspace_id,profile_id,import_id,document_id,line_index,booking_date,amount,currency,account_id,dedupe_group,dedupe_key,created_at) VALUES('line','W','P','imp','doc',1,'2025-03-03','-1000.00','EUR','OWNACCOUNT','g-k','k','2025');
INSERT INTO source_funds_sources(id,workspace_id,profile_id,source_type,label,asset,amount,created_at,updated_at) VALUES('src','W','P','fiat_purchase','Bank-funded purchase','BTC',1,'2025','2025');
INSERT INTO bank_statement_line_decisions(line_id,transaction_id,profile_id,state,source_id,created_at,updated_at) VALUES('line','tx','P','accepted','src','2025','2025');"
        .replace("'W'", &format!("'{wid}'"))
        .replace("'P'", &format!("'{pid}'"));
    conn.execute_batch(&sql).unwrap();
    (wid, pid)
}

const TABLES: [&str; 3] = [
    "bank_statement_imports",
    "bank_statement_lines",
    "bank_statement_line_decisions",
];

#[test]
fn wipe_everything_discards_bank_statements_and_their_documents() {
    let f = Fixture::seed();
    seed_bank(&f);
    let conn = f.book.connection();
    let attachments = f._root.path().join("attachments");
    let result = profile_reset::execute(
        conn,
        &dict([("wipe_everything", Value::Bool(true))]),
        &attachments,
        &f.sources,
    )
    .unwrap();
    for table in TABLES {
        assert_eq!(
            scalar(conn, &format!("SELECT COUNT(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    assert_eq!(
        scalar(
            conn,
            "SELECT COUNT(*) FROM external_documents WHERE id='doc'"
        ),
        0,
        "the statement document goes with its import"
    );
    let removed = get(&result, "removed").unwrap();
    assert_eq!(
        get(removed, "bank_statement_imports"),
        Some(&Value::Int(1.into()))
    );
    assert_eq!(
        get(removed, "bank_statement_lines"),
        Some(&Value::Int(1.into()))
    );
    // Nothing is left for a re-import to find as already imported.
    assert_eq!(
        scalar(
            conn,
            "SELECT COUNT(*) FROM bank_statement_imports WHERE file_sha256='sha-of-the-file'"
        ),
        0
    );
}

#[test]
fn keep_decisions_reset_keeps_bank_statements() {
    let f = Fixture::seed();
    seed_bank(&f);
    let conn = f.book.connection();
    let attachments = f._root.path().join("attachments");
    profile_reset::execute(conn, &dict::<&str>([]), &attachments, &f.sources).unwrap();
    for table in TABLES {
        assert!(
            scalar(conn, &format!("SELECT COUNT(*) FROM {table}")) >= 1,
            "{table}"
        );
    }
}

#[test]
fn a_book_without_bank_tables_wipes_exactly_as_before() {
    let f = Fixture::seed();
    let conn = f.book.connection();
    let attachments = f._root.path().join("attachments");
    let result = profile_reset::execute(
        conn,
        &dict([("wipe_everything", Value::Bool(true))]),
        &attachments,
        &f.sources,
    )
    .unwrap();
    assert!(get(get(&result, "removed").unwrap(), "bank_statement_lines").is_none());
    assert!(!kassiber_store::optional::bank_statements_present(conn).unwrap());
}

/// A keep-decisions reset would replace a 'changed' reservation with the
/// changed transaction's facts, and the next resync would settle it and
/// release the bank decisions it withholds without a review. Such a reset is
/// refused until those lines are reviewed again; reviewing one admits it.
#[test]
fn keep_decisions_reset_refuses_while_a_reservation_withholds_bank_decisions() {
    let f = Fixture::seed();
    let (wid, pid) = seed_bank(&f);
    let conn = f.book.connection();
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wallet',?,?,'Wallet','descriptor','2025')",
        [&wid, &pid],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO book_reset_transactions(transaction_id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,amount_includes_fee,metadata_json,state,changed_fields_json,created_at) VALUES('tx',?,?,'wallet','fp','2025-03-05T10:00:00Z','inbound','BTC',1,0,0,'{}','changed','[\"amount\"]','2026')",
        [&wid, &pid],
    )
    .unwrap();
    let attachments = f._root.path().join("attachments");
    let refused =
        profile_reset::execute(conn, &dict::<&str>([]), &attachments, &f.sources).unwrap_err();
    assert_eq!(
        refused.code, "book_reset_withheld_bank_decisions",
        "{refused:?}"
    );
    assert_eq!(
        scalar(
            conn,
            "SELECT COUNT(*) FROM book_reset_transactions WHERE state='changed'"
        ),
        1,
        "nothing was reset"
    );
    // Reviewing the line again (a decision newer than the reservation)
    // admits the reset.
    conn.execute(
        "UPDATE bank_statement_line_decisions SET updated_at='2027'",
        [],
    )
    .unwrap();
    profile_reset::execute(conn, &dict::<&str>([]), &attachments, &f.sources).unwrap();
}
