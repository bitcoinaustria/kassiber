//! Accepting bank-statement lines through the source-funds mutations, and
//! their disclosure in the report. Synthetic names and IBANs only.
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;
use kassiber_reports::source_funds_bank;
use kassiber_setup::SystemSources;
use kassiber_store::{rusqlite::Connection, Book};
use kassiber_transactions::{
    bank_statements::{
        matching,
        store::{self, ImportRequest},
        ParseOptions,
    },
    Scope,
};

fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// A book with three Kraken buys, a deposit, and an imported statement.
fn book() -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    conn.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z'); INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Main','EUR','2025-01-01T00:00:00Z'); INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('kraken','w','p','Kraken','kraken','2025-01-01T00:00:00Z'),('cold','w','p','Cold','descriptor','2025-01-01T00:00:00Z'); INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,kind,fiat_currency,fiat_value_exact,created_at) VALUES('buy1','w','p','kraken','f1','BTC','inbound',2000000000,0,'2025-03-05T10:00:00Z','buy','EUR','1000.00','2025-01-01T00:00:00Z'),('buy2','w','p','kraken','f2','BTC','inbound',1000000000,0,'2025-03-06T10:00:00Z','buy','CHF','500.00','2025-01-01T00:00:00Z'),('buy3','w','p','kraken','f3','BTC','inbound',1000000000,0,'2025-03-06T11:00:00Z','buy','EUR','700.00','2025-01-01T00:00:00Z'),('dep','w','p','cold','f4','BTC','inbound',1000000000,0,'2025-03-06T11:00:00Z','deposit','EUR','700.00','2025-01-01T00:00:00Z');").unwrap();
    let file = dir.path().join("statement.csv");
    let csv = "Buchungsdatum;Empfänger;IBAN;Verwendungszweck;Bankreferenz;Betrag;Währung\n\
         03.03.2025;Payward Ltd;AT61 1904 3002 3457 3201;Kraken deposit 7;REF-7;-1.000,00;EUR\n\
         05.03.2025;Example Employer AG;;Gehalt;REF-8;2.500,00;EUR\n";
    std::fs::write(&file, csv).unwrap();
    let options = ParseOptions {
        account: Some("DE89370400440532013000".into()),
        ..ParseOptions::default()
    };
    let request = ImportRequest {
        file: file.to_string_lossy().into_owned(),
        options,
        label: None,
        dry_run: false,
    };
    store::import_statement(conn, &scope(), &dir.path().join("attachments"), &request).unwrap();
    (dir, book)
}

fn line(conn: &Connection, amount: &str) -> String {
    conn.query_row(
        "SELECT id FROM bank_statement_lines WHERE amount=?",
        [amount],
        |r| r.get(0),
    )
    .unwrap()
}

fn report(conn: &Connection, mode: &str) -> String {
    let args = dict([
        ("workspace", s("w")),
        ("profile", s("p")),
        ("target_transaction", s("buy1")),
        ("reveal_mode", s(mode)),
    ]);
    format!(
        "{:?}",
        kassiber_reports::execute(conn, "reports.source-funds", &args).unwrap()
    )
}

#[test]
fn accepting_creates_a_reviewed_fiat_source_and_reports_the_line() {
    let (_dir, book) = book();
    let conn = book.connection();
    let lines = vec![line(conn, "-1000.00")];
    let accepted =
        source_funds_bank::accept(conn, &scope(), &lines, &["buy1".into()], &SystemSources)
            .unwrap();
    let first = match get(&accepted, "accepted") {
        Some(Value::List(items)) => items[0].clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(get(&first, "source_created"), Some(&Value::Bool(true)));
    let (description, method): (String, String) = conn
        .query_row("SELECT s.description, l.method FROM source_funds_sources s JOIN source_funds_links l ON l.from_source_id=s.id", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(description, "Paid by bank transfer: 2025-03-03 -1000.00 EUR to Kraken (account ••3000). Bank statement evidence only.");
    assert_eq!(method, "bank_statement");
    let reasons: String = conn
        .query_row(
            "SELECT reasons_json FROM bank_statement_line_decisions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        reasons.contains("bank amount equals the purchase value 1000.00"),
        "{reasons}"
    );
    assert_eq!(
        count(conn, "SELECT COUNT(*) FROM journal_entries"),
        0,
        "evidence, never accounting"
    );
    // Accepting again reuses the source; no statement file is copied.
    source_funds_bank::accept(conn, &scope(), &lines, &["buy1".into()], &SystemSources).unwrap();
    assert_eq!(count(conn, "SELECT COUNT(*) FROM source_funds_sources"), 1);
    assert_eq!(count(conn, "SELECT COUNT(*) FROM attachments"), 1);
    // The report discloses the line by mode, never the remittance text.
    let standard = report(conn, "standard");
    assert!(
        standard.contains("bank_evidence")
            && standard.contains("Payward Ltd")
            && standard.contains("AT** **** **** **** 3201"),
        "{standard}"
    );
    assert!(!standard.contains("Kraken deposit 7") && !standard.contains("AT611904300234573201"));
    let labels = report(conn, "labels_only");
    assert!(
        labels.contains("-1000.00") && !labels.contains("Payward") && !labels.contains("3201"),
        "{labels}"
    );
    // Rejecting withdraws the evidence and keeps the reviewed source.
    let rejected = matching::reject(conn, &scope(), &lines, &["buy1".into()]).unwrap();
    assert_eq!(
        get(&rejected, "withdrawn_accepted"),
        Some(&Value::Int(1.into()))
    );
    assert!(!report(conn, "standard").contains("bank_evidence"));
    assert_eq!(count(conn, "SELECT COUNT(*) FROM source_funds_sources"), 1);
}

/// A purchase whose reviewed fiat source the owner already linked keeps
/// that one link: accepting bank evidence attaches to it instead of adding
/// a second full allocation the report would count twice.
#[test]
fn accepting_reuses_the_reviewed_link_the_purchase_already_has() {
    let (_dir, book) = book();
    let conn = book.connection();
    let execute = |kind: &str, args: Vec<(&str, Value)>| {
        let mut args = args;
        args.extend([("workspace", s("w")), ("profile", s("p"))]);
        kassiber_reports::execute(conn, kind, &dict(args)).unwrap()
    };
    let source = execute(
        "source-funds.sources.create",
        vec![
            ("source_type", s("fiat_purchase")),
            ("label", s("Kraken purchase")),
            ("asset", s("BTC")),
            ("amount", s("0.02")),
            ("fiat_currency", s("EUR")),
            ("acquired_at", s("2025-03-05T10:00:00Z")),
            ("attachments", Value::List(vec![])),
        ],
    );
    let link = execute(
        "source-funds.links.create",
        vec![
            ("from_source", get(&source, "id").unwrap().clone()),
            ("to_transaction", s("buy1")),
            ("link_type", s("manual_source")),
            ("state", s("reviewed")),
            ("confidence", s("strong")),
            ("method", s("manual")),
            ("allocation_policy", s("explicit")),
            ("allocation_amount", s("0.02")),
            ("attachments", Value::List(vec![])),
        ],
    );
    let lines = vec![line(conn, "-1000.00")];
    let accepted =
        source_funds_bank::accept(conn, &scope(), &lines, &["buy1".into()], &SystemSources)
            .unwrap();
    let first = match get(&accepted, "accepted") {
        Some(Value::List(items)) => items[0].clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(get(&first, "link_id"), get(&link, "id"));
    assert_eq!(get(&first, "source_created"), Some(&Value::Bool(false)));
    assert_eq!(count(conn, "SELECT COUNT(*) FROM source_funds_links"), 1);
    let standard = report(conn, "standard");
    assert!(standard.contains("bank_evidence"), "{standard}");
    assert!(!standard.contains("source_overallocation"), "{standard}");
}

#[test]
fn accept_refuses_pairs_that_cannot_be_funding_and_writes_nothing() {
    let (_dir, book) = book();
    let conn = book.connection();
    let refuse = |amount: &str, tx: &str| {
        let lines = vec![if amount.is_empty() {
            "nope".to_owned()
        } else {
            line(conn, amount)
        }];
        source_funds_bank::accept(conn, &scope(), &lines, &[tx.to_owned()], &SystemSources)
            .unwrap_err()
    };
    assert!(refuse("2500.00", "buy1").message.contains("incoming"));
    assert!(refuse("-1000.00", "dep")
        .message
        .contains("not an inbound BTC purchase"));
    assert!(refuse("-1000.00", "buy2").message.contains("priced in CHF"));
    assert_eq!(refuse("", "buy1").code, "not_found");
    conn.execute_batch("INSERT INTO source_funds_sources(id,workspace_id,profile_id,source_type,label,asset,amount,review_state,created_at,updated_at) VALUES('gift','w','p','gift','Gift','BTC',1000000000,'reviewed','2025','2025'); INSERT INTO source_funds_links(id,workspace_id,profile_id,from_source_id,to_transaction_id,link_type,state,asset,created_at,updated_at) VALUES('gl','w','p','gift','buy3','manual_source','reviewed','BTC','2025','2025');").unwrap();
    assert_eq!(refuse("-1000.00", "buy3").code, "conflict");
    assert_eq!(
        count(conn, "SELECT COUNT(*) FROM bank_statement_line_decisions"),
        0
    );
    assert_eq!(count(conn, "SELECT COUNT(*) FROM source_funds_sources"), 1);
}

/// Accepting a displayed suggestion binds to it: once the pair is no longer
/// suggested (the purchase's value changed, or a competing line arrived), the
/// acceptance is refused for a new review instead of becoming an explicit
/// pairing that never met the suggestion's conditions.
#[test]
fn accepting_a_suggestion_refuses_once_it_is_no_longer_suggested() {
    let (_dir, book) = book();
    let conn = book.connection();
    let lines = vec![line(conn, "-1000.00")];
    let buys = vec!["buy1".to_owned()];
    let id = matching::group_key(&lines, &buys);
    let accept = |suggestion: &str| {
        source_funds_bank::accept_suggestion(
            conn,
            &scope(),
            &lines,
            &buys,
            suggestion,
            &SystemSources,
        )
    };
    // The id must be the named pair's own suggestion.
    assert_eq!(accept("0000000000000000").unwrap_err().code, "validation");
    conn.execute(
        "UPDATE transactions SET fiat_value_exact='1001.00' WHERE id='buy1'",
        [],
    )
    .unwrap();
    let stale = accept(&id).unwrap_err();
    assert_eq!(stale.code, "conflict", "{stale:?}");
    assert_eq!(
        count(conn, "SELECT COUNT(*) FROM bank_statement_line_decisions"),
        0
    );
    assert_eq!(count(conn, "SELECT COUNT(*) FROM source_funds_sources"), 0);
    // Still suggested: accepted with the suggestion's reasons.
    conn.execute(
        "UPDATE transactions SET fiat_value_exact='1000.00' WHERE id='buy1'",
        [],
    )
    .unwrap();
    accept(&id).unwrap();
    let reasons: String = conn
        .query_row(
            "SELECT reasons_json FROM bank_statement_line_decisions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(reasons.contains("equals the purchase value"), "{reasons}");
}
