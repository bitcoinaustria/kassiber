use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;
use kassiber_store::{
    rusqlite::{params, Connection},
    Book,
};
use kassiber_transactions::{
    commercial,
    documents::{self, Command},
    Scope,
};
use std::{fs, path::Path};
fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}
fn book() -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    book.connection().execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025'); INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','Main','2025'),('q','w','Other','2025'); INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wallet','w','p','Wallet','custom','2025'); INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,payment_hash,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,created_at) VALUES('tx','w','p','wallet','tx','chain','hash','BTC','outbound',1101,100,1,'2025-01-01T00:00:00Z','withdrawal','2025');").unwrap();
    (dir, book)
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    match get(v, key).unwrap() {
        Value::Str(t) => t.as_str().unwrap(),
        _ => panic!("{key}: {v:?}"),
    }
}
fn v(v: &Value, key: &str) -> Value {
    get(v, key).unwrap().clone()
}
fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(v) => v,
        _ => panic!("{v:?}"),
    }
}
fn create(conn: &Connection, label: &str, reference: Option<&str>) -> Value {
    documents::execute(
        conn,
        &scope(),
        Path::new("unused"),
        &Command::Create(dict([
            ("document_type", s("invoice")),
            ("label", s(label)),
            ("external_ref", reference.map_or(Value::Null, s)),
        ])),
    )
    .unwrap()
}
fn suggest(conn: &Connection) -> Value {
    commercial::execute(conn, &scope(), &commercial::Command::Suggest { limit: 500 }).unwrap()
}
fn context(conn: &Connection) -> Value {
    commercial::execute(
        conn,
        &scope(),
        &commercial::Command::Context {
            transaction: "tx".into(),
        },
    )
    .unwrap()
}
fn record(
    conn: &Connection,
    id: &str,
    kind: &str,
    txid: Option<&str>,
    hash: Option<&str>,
    amount: Option<i64>,
    reference: &str,
) {
    conn.execute("INSERT INTO btcpay_provenance_records(id,workspace_id,profile_id,record_type,stable_key,backend_name,store_id,invoice_id,txid,payment_hash,amount,asset,status,fiat_currency,fiat_value_exact,created_at,updated_at) VALUES(?,'w','p',?,?, 'Merchant','store',?,?,?,?,'BTC','Completed','EUR','12.00','2025','2025')",params![id,kind,format!("stable-{id}"),reference,txid,hash,amount]).unwrap();
}
#[test]
fn document_registration_preserves_exact_values_refusals_scope_and_order() {
    let (_dir, book) = book();
    let conn = book.connection();
    let doc = documents::execute(
        conn,
        &scope(),
        Path::new("unused"),
        &Command::Create(dict([
            ("document_type", s(" TAX-REPORT ")),
            ("label", s("  literal  ")),
            ("external_ref", s("ref")),
            ("fiat_currency", s(" eur ")),
            ("fiat_value", s("1.230000000000000000000000000000000001E+2")),
            ("issued_at", s("2025-01-01T01:00:00.999+01:00")),
            ("due_at", s("1970-01-01")),
        ])),
    )
    .unwrap();
    assert_eq!(text(&doc, "document_type"), "tax_report");
    assert_eq!(text(&doc, "label"), "  literal  ");
    assert_eq!(text(&doc, "fiat_currency"), " EUR ");
    assert_eq!(
        text(&doc, "fiat_value_exact"),
        "123.0000000000000000000000000000000001"
    );
    assert_eq!(text(&doc, "issued_at"), "2025-01-01T00:00:00Z");
    assert_eq!(text(&doc, "due_at"), "");
    let duplicate = documents::execute(
        conn,
        &scope(),
        Path::new("unused"),
        &Command::Create(dict([
            ("document_type", s("receipt")),
            ("label", s("other")),
            ("external_ref", s("ref")),
        ])),
    )
    .unwrap_err();
    assert_eq!(duplicate.code, "conflict");
    let a = create(conn, "same", Some("a"));
    let b = create(conn, "same", Some("b"));
    assert_eq!(
        documents::get_document(conn, "p", "same").unwrap_err().code,
        "ambiguous"
    );
    assert_eq!(
        documents::get_document(conn, "q", text(&a, "id"))
            .unwrap_err()
            .code,
        "not_found"
    );
    conn.execute(
        "UPDATE external_documents SET label=? WHERE id=?",
        [text(&a, "id"), text(&b, "id")],
    )
    .unwrap();
    assert_eq!(
        documents::get_document(conn, "p", text(&a, "id")).unwrap(),
        a
    );
    conn.execute(
        "UPDATE external_documents SET created_at='2025',updated_at='2025'",
        [],
    )
    .unwrap();
    let all = documents::execute(
        conn,
        &scope(),
        Path::new("unused"),
        &Command::List { limit: 0 },
    )
    .unwrap();
    let ids: Vec<_> = list(&all).iter().map(|r| text(r, "id")).collect();
    assert!(ids.windows(2).all(|w| w[0] > w[1]));
    let one = documents::execute(
        conn,
        &scope(),
        Path::new("unused"),
        &Command::List { limit: -10 },
    )
    .unwrap();
    assert_eq!(list(&one).len(), 1);
    for (key, input, code) in [
        ("document_type", "no", "validation"),
        ("issued_at", "invalid", "validation"),
        ("fiat_value", "no", "validation"),
        ("fiat_value", "--Infinity", "validation"),
        ("issued_at", "0001-01-01T00:00:00+01:00", "internal_error"),
    ] {
        let mut args = dict([("document_type", s("invoice")), ("label", s("Invalid"))]);
        if let Value::Dict(p) = &mut args {
            p.retain(|(k, _)| *k != key.into());
            p.push((key.into(), s(input)));
        }
        assert_eq!(
            documents::execute(conn, &scope(), Path::new("unused"), &Command::Create(args))
                .unwrap_err()
                .code,
            code
        );
    }
    assert_eq!(
        list(
            &documents::execute(
                conn,
                &scope(),
                Path::new("unused"),
                &Command::List { limit: 100 }
            )
            .unwrap()
        )
        .len(),
        3
    );
}
#[test]
fn document_prices_are_bounded_finite_and_canonicalize_zero() {
    let (_dir, book) = book();
    let conn = book.connection();
    for (raw, message) in [
        ("bad", "commercial price must be a decimal number"),
        ("NaN", "commercial price must be a finite decimal number"),
        (
            "Infinity",
            "commercial price must be a finite decimal number",
        ),
        ("1e309", "commercial price must be a finite decimal number"),
        (
            "1e-14",
            "commercial price is outside the supported decimal range",
        ),
    ] {
        let err = documents::execute(
            conn,
            &scope(),
            Path::new("unused"),
            &Command::Create(dict([
                ("document_type", s("invoice")),
                ("label", s("Invalid")),
                ("fiat_value", s(raw)),
            ])),
        )
        .unwrap_err();
        assert_eq!(err.code, "validation");
        assert_eq!(err.message, message);
    }
    let malformed = Value::Str(kassiber_pyjson::PyStr::from_code_points([49, 0xd800]).unwrap());
    let err = documents::execute(
        conn,
        &scope(),
        Path::new("unused"),
        &Command::Create(dict([
            ("document_type", s("invoice")),
            ("label", s("Invalid")),
            ("fiat_value", malformed),
        ])),
    )
    .unwrap_err();
    assert_eq!(err.code, "validation");
    assert_eq!(err.message, "commercial price must be a decimal number");
    for (raw, expected) in [
        ("-0e-999", "0"),
        ("12.500", "12.500"),
        ("1e-13", "0.0000000000001"),
    ] {
        let doc = documents::execute(
            conn,
            &scope(),
            Path::new("unused"),
            &Command::Create(dict([
                ("document_type", s("invoice")),
                ("label", s("Bounded")),
                ("fiat_value", s(raw)),
            ])),
        )
        .unwrap();
        assert_eq!(text(&doc, "fiat_value_exact"), expected);
    }
}

#[test]
fn document_evidence_is_literal_transactionless_hashed_and_atomic() {
    let (dir, book) = book();
    let conn = book.connection();
    let root = dir.path().join("attachments");
    let doc = create(conn, "invoice", None);
    let source = dir.path().join("invoice.txt");
    fs::write(&source, b"synthetic evidence\n").unwrap();
    let attachment = documents::execute(
        conn,
        &scope(),
        &root,
        &Command::Attach(dict([
            ("document", v(&doc, "id")),
            ("file", s(source.to_str().unwrap())),
        ])),
    )
    .unwrap();
    let (tx, relative, sha, size): (Option<String>, String, String, i64) = conn
        .query_row(
            "SELECT transaction_id,stored_relpath,sha256,size_bytes FROM attachments WHERE id=?",
            [text(&attachment, "attachment_id")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(tx, None);
    assert_eq!(
        relative,
        format!("p/{}-invoice.txt", text(&attachment, "attachment_id"))
    );
    assert_eq!(size, 19);
    assert_eq!(
        sha,
        "b781ca4e3efcbf31178927825de855f45b7e52f0d652af03cfb236785de2fe48"
    );
    assert_eq!(
        fs::read(root.join(relative)).unwrap(),
        b"synthetic evidence\n"
    );
    let literal = "  no-scheme://127.0.0.1:1/path\n";
    let url = documents::execute(
        conn,
        &scope(),
        &root,
        &Command::Attach(dict([
            ("document", v(&doc, "id")),
            ("url", s(literal)),
            ("label", s("   ")),
        ])),
    )
    .unwrap();
    assert_eq!(text(&url, "label"), "   ");
    let stored: String = conn
        .query_row(
            "SELECT source_url FROM attachments WHERE id=?",
            [text(&url, "attachment_id")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, literal);
    let unencodable =
        Value::Str(kassiber_pyjson::PyStr::from_code_points([b'x' as u32, 0xd800]).unwrap());
    assert_eq!(
        documents::execute(
            conn,
            &scope(),
            &root,
            &Command::Attach(dict([("document", v(&doc, "id")), ("url", unencodable),])),
        )
        .unwrap_err()
        .code,
        "internal_error"
    );
    conn.execute_batch("CREATE TRIGGER fail_document_evidence BEFORE INSERT ON external_document_attachments BEGIN SELECT RAISE(ABORT,'failed evidence join'); END;").unwrap();
    let before = fs::read_dir(root.join("p")).unwrap().count();
    assert_eq!(
        documents::execute(
            conn,
            &scope(),
            &root,
            &Command::Attach(dict([
                ("document", v(&doc, "id")),
                ("file", s(source.to_str().unwrap()))
            ]))
        )
        .unwrap_err()
        .code,
        "internal_error"
    );
    assert_eq!(fs::read_dir(root.join("p")).unwrap().count(), before);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM attachments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn prior_reports_are_atomic_evidence_without_ledger_or_lot_authority() {
    let (dir, book) = book();
    let conn = book.connection();
    let root = dir.path().join("attachments");
    let source = dir.path().join("prior.pdf");
    fs::write(&source, b"prior report").unwrap();
    let args = dict([
        ("provider", s("  Calculator  ")),
        ("tax_year", s(" 2024 ")),
        ("file", s(source.to_str().unwrap())),
    ]);
    let result =
        documents::execute(conn, &scope(), &root, &Command::ImportReport(args.clone())).unwrap();
    let doc = get(&result, "document").unwrap();
    assert_eq!(text(doc, "document_type"), "tax_report");
    assert_eq!(text(doc, "issuer"), "Calculator");
    assert_eq!(text(doc, "label"), "Calculator 2024 tax report");
    assert_eq!(text(doc, "issued_at"), "2024-12-31T00:00:00Z");
    assert!(text(doc, "notes").contains("evidence only"));
    let raw: String = conn
        .query_row("SELECT raw_json FROM external_documents", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        raw,
        "{\"provider\": \"Calculator\", \"tax_year\": \"2024\"}"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM journal_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT kind FROM transactions", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "withdrawal"
    );
    conn.execute_batch("CREATE TRIGGER fail_report BEFORE INSERT ON external_document_attachments BEGIN SELECT RAISE(ABORT,'fail'); END;").unwrap();
    let before = fs::read_dir(root.join("p")).unwrap().count();
    assert!(documents::execute(conn, &scope(), &root, &Command::ImportReport(args)).is_err());
    assert_eq!(fs::read_dir(root.join("p")).unwrap().count(), before);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM external_documents", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(
        conn,
        "payment",
        "payment",
        Some("chain"),
        None,
        Some(1001),
        "inv",
    );
    record(
        conn,
        "mismatch",
        "payment",
        None,
        Some("hash"),
        Some(1101),
        "inv",
    );
    record(conn, "stable", "payment", None, None, None, "inv");
    record(conn, "no-signal", "payment", None, None, Some(1001), "inv");
    conn.execute("UPDATE transactions SET external_id='stable-stable'", [])
        .unwrap();
    let first = suggest(conn);
    let links = list(get(&first, "suggestions").unwrap());
    assert_eq!(links.len(), 2);
    assert_eq!(
        text(
            links
                .iter()
                .find(|l| text(l, "btcpay_record_id") == "mismatch")
                .unwrap(),
            "confidence"
        ),
        "strong"
    );
    assert_eq!(
        text(
            links
                .iter()
                .find(|l| text(l, "btcpay_record_id") == "stable")
                .unwrap(),
            "method"
        ),
        "txid_or_payment_hash"
    );
    conn.execute("UPDATE transactions SET external_id='chain'", [])
        .unwrap();
    let second = suggest(conn);
    let links = list(get(&second, "suggestions").unwrap());
    assert_eq!(links.len(), 2);
    assert_eq!(
        text(
            links
                .iter()
                .find(|l| text(l, "btcpay_record_id") == "payment")
                .unwrap(),
            "confidence"
        ),
        "exact"
    );
    conn.execute(
        "DELETE FROM commercial_links WHERE btcpay_record_id='payment'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE transactions SET direction='inbound',amount=-1001",
        [],
    )
    .unwrap();
    let negative = suggest(conn);
    assert_eq!(
        text(
            list(get(&negative, "suggestions").unwrap())
                .iter()
                .find(|l| text(l, "btcpay_record_id") == "payment")
                .unwrap(),
            "confidence"
        ),
        "exact"
    );
    conn.execute(
        "UPDATE transactions SET direction='outbound',amount=1101",
        [],
    )
    .unwrap();
    conn.execute("UPDATE transactions SET excluded=1", [])
        .unwrap();
    assert!(list(get(&suggest(conn), "suggestions").unwrap()).is_empty());
    conn.execute("UPDATE transactions SET excluded=0", [])
        .unwrap();
    conn.execute_batch("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,payment_hash,asset,direction,amount,occurred_at,kind,created_at) SELECT 'duplicate',workspace_id,profile_id,wallet_id,'duplicate',external_id,payment_hash,asset,direction,amount,occurred_at,kind,created_at FROM transactions;").unwrap();
    assert!(list(get(&suggest(conn), "suggestions").unwrap()).is_empty());
    conn.execute(
        "UPDATE transactions SET profile_id='q' WHERE id='duplicate'",
        [],
    )
    .unwrap();
    assert_eq!(list(get(&suggest(conn), "suggestions").unwrap()).len(), 2);
    assert_eq!(
        conn.query_row("SELECT kind FROM transactions WHERE id='tx'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "withdrawal"
    );
    assert_eq!(
        conn.query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn commercial_payout_matching_requires_outgoing_proof_and_matchable_status() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(
        conn,
        "payout",
        "payout",
        Some("chain"),
        None,
        Some(1001),
        "inv",
    );
    assert_eq!(
        text(
            &list(get(&suggest(conn), "suggestions").unwrap())[0],
            "method"
        ),
        "payout_proof"
    );
    conn.execute("UPDATE transactions SET direction='inbound'", [])
        .unwrap();
    assert!(list(get(&suggest(conn), "suggestions").unwrap()).is_empty());
    conn.execute("UPDATE transactions SET direction='outbound'", [])
        .unwrap();
    conn.execute("UPDATE btcpay_provenance_records SET status='Awaiting'", [])
        .unwrap();
    assert!(list(get(&suggest(conn), "suggestions").unwrap()).is_empty());
    conn.execute(
        "UPDATE btcpay_provenance_records SET status='InProgress'",
        [],
    )
    .unwrap();
    assert_eq!(list(get(&suggest(conn), "suggestions").unwrap()).len(), 1);
}
#[test]
fn document_matching_enriches_identity_preserves_review_and_rejection_and_deduplicates() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(
        conn,
        "payment",
        "payment",
        Some("chain"),
        None,
        Some(1001),
        "invoice",
    );
    record(conn, "invoice", "invoice", None, None, None, "invoice");
    let first = suggest(conn);
    let id = text(&list(get(&first, "suggestions").unwrap())[0], "id").to_owned();
    conn.execute("UPDATE commercial_links SET state='reviewed',commercial_kind='expense',reviewed_at='2025',applied_transaction_snapshot_json='{}' WHERE id=?",[&id]).unwrap();
    let doc = create(conn, "Invoice", Some("invoice"));
    conn.execute(
        "UPDATE external_documents SET fiat_value_exact='17.20' WHERE id=?",
        [text(&doc, "id")],
    )
    .unwrap();
    let result = suggest(conn);
    assert_eq!(v(&result, "created"), Value::Int(1.into()));
    let links = list(get(&result, "suggestions").unwrap());
    assert_eq!(links.len(), 2);
    let payment = links.iter().find(|l| text(l, "id") == id).unwrap();
    assert_eq!(v(payment, "document_id"), v(&doc, "id"));
    assert_eq!(text(payment, "allocation_fiat_exact"), "17.20");
    assert_eq!(text(payment, "state"), "reviewed");
    assert_eq!(text(payment, "confidence"), "exact");
    assert_eq!(text(payment, "commercial_kind"), "expense");
    assert_eq!(
        text(payment, "method"),
        "document_btcpay_txid_or_payment_hash"
    );
    let other = create(conn, "Other", Some("other"));
    conn.execute(
        "UPDATE external_documents SET fiat_currency='EUR',fiat_value_exact='12.00' WHERE id=?",
        [text(&other, "id")],
    )
    .unwrap();
    let result = suggest(conn);
    let links = list(get(&result, "suggestions").unwrap());
    assert_eq!(links.iter().filter(|l| text(l, "id") == id).count(), 1);
    assert_eq!(
        text(
            links.iter().find(|l| text(l, "id") == id).unwrap(),
            "document_id"
        ),
        text(&doc, "id")
    );
    conn.execute(
        "UPDATE commercial_links SET state='rejected' WHERE id=?",
        [&id],
    )
    .unwrap();
    let result = suggest(conn);
    assert_eq!(v(&result, "created"), Value::Int(1.into()));
    assert!(list(get(&result, "suggestions").unwrap())
        .iter()
        .all(|l| text(l, "id") != id));
    assert_eq!(v(&suggest(conn), "created"), Value::Int(0.into()));
}
#[test]
fn document_matching_uses_reference_or_exact_currency_value_and_requires_unique_payment() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(
        conn,
        "payment",
        "payment",
        Some("chain"),
        None,
        Some(1001),
        "different",
    );
    record(conn, "invoice", "invoice", None, None, None, "different");
    let doc = create(conn, "Invoice", Some("request"));
    conn.execute(
        "UPDATE btcpay_provenance_records SET payment_request_id='request'",
        [],
    )
    .unwrap();
    assert_eq!(list(get(&suggest(conn), "suggestions").unwrap()).len(), 2);
    conn.execute(
        "UPDATE btcpay_provenance_records SET payment_request_id=NULL,order_id='request'",
        [],
    )
    .unwrap();
    assert_eq!(list(get(&suggest(conn), "suggestions").unwrap()).len(), 2);
    conn.execute("UPDATE btcpay_provenance_records SET order_id=NULL", [])
        .unwrap();
    assert_eq!(list(get(&suggest(conn), "suggestions").unwrap()).len(), 1);
    conn.execute(
        "UPDATE external_documents SET fiat_currency='EUR',fiat_value_exact='12.00'",
        [],
    )
    .unwrap();
    assert_eq!(list(get(&suggest(conn), "suggestions").unwrap()).len(), 2);
    conn.execute("UPDATE external_documents SET fiat_value_exact='12.0'", [])
        .unwrap();
    assert_eq!(list(get(&suggest(conn), "suggestions").unwrap()).len(), 1);
    conn.execute(
        "UPDATE external_documents SET fiat_value_exact='12.00',fiat_currency='USD'",
        [],
    )
    .unwrap();
    assert_eq!(list(get(&suggest(conn), "suggestions").unwrap()).len(), 1);
    conn.execute("UPDATE external_documents SET external_ref='different'", [])
        .unwrap();
    conn.execute("UPDATE btcpay_provenance_records SET profile_id='q'", [])
        .unwrap();
    assert!(list(get(&suggest(conn), "suggestions").unwrap()).is_empty());
    assert_eq!(
        v(
            &documents::get_document(conn, "p", text(&doc, "id")).unwrap(),
            "label"
        ),
        s("Invoice")
    );
}
#[test]
fn commercial_context_keeps_order_invoice_preference_redaction_boundary_and_stored_freshness() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(
        conn,
        "payment",
        "payment",
        Some("chain"),
        None,
        Some(1001),
        "invoice",
    );
    record(conn, "invoice", "invoice", None, None, None, "invoice");
    let doc = create(conn, "Invoice", Some("invoice"));
    suggest(conn);
    conn.execute("UPDATE btcpay_provenance_records SET payment_request_id='request',origin_kind='payment_request',origin_label='Request',origin_url='http://127.0.0.1:1/literal',fiat_currency='BTC' WHERE id='invoice'",[]).unwrap();
    conn.execute(
        "UPDATE commercial_links SET reviewed_at='2025',state='reviewed' WHERE transaction_id='tx'",
        [],
    )
    .unwrap();
    let before: i64 = conn
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    let ctx = context(conn);
    assert_eq!(v(&ctx, "transaction_id"), s("tx"));
    assert_eq!(list(get(&ctx, "documents").unwrap()).len(), 1);
    assert_eq!(
        v(&list(get(&ctx, "documents").unwrap())[0], "id"),
        v(&doc, "id")
    );
    let matching = &list(get(&ctx, "btcpay").unwrap())[0];
    assert_eq!(
        text(get(matching, "payment_request").unwrap(), "id"),
        "request"
    );
    assert_eq!(
        text(get(matching, "origin").unwrap(), "url"),
        "http://127.0.0.1:1/literal"
    );
    assert_eq!(
        text(get(matching, "invoice").unwrap(), "fiat_value_exact"),
        ""
    );
    let fresh = &list(get(&ctx, "btcpay_freshness").unwrap())[0];
    assert_eq!(v(fresh, "stale"), Value::Bool(true));
    assert_eq!(v(fresh, "never_synced"), Value::Bool(true));
    assert_eq!(
        conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    conn.execute("INSERT INTO btcpay_store_sync_states(profile_id,backend_name,store_id,last_success_at,last_attempt_at,updated_at) VALUES('p','merchant','store','9999-01-01','9999-01-01','9999-01-01')",[]).unwrap();
    let fresh_ctx = context(conn);
    let fresh = &list(get(&fresh_ctx, "btcpay_freshness").unwrap())[0];
    assert_eq!(v(fresh, "age_seconds"), Value::Int(0.into()));
    assert_eq!(v(fresh, "stale"), Value::Bool(false));
    conn.execute("UPDATE commercial_links SET state='rejected'", [])
        .unwrap();
    assert!(list(get(&context(conn), "links").unwrap()).is_empty());
}
#[test]
fn commercial_context_payout_batches_preserve_exact_sums_and_currency_barriers() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(conn, "a", "payout", Some("chain"), None, Some(-1001), "a");
    record(conn, "b", "payout", Some("chain"), None, Some(2002), "b");
    suggest(conn);
    conn.execute("UPDATE btcpay_provenance_records SET fiat_value_exact=CASE id WHEN 'a' THEN '123456789012345678901234567890.12' ELSE '0.12' END",[]).unwrap();
    let ctx = context(conn);
    let batch = get(&ctx, "payout_batch").unwrap();
    assert_eq!(v(batch, "amount_msat"), Value::Int(3003.into()));
    assert_eq!(
        text(batch, "fiat_value_exact"),
        "123456789012345678901234567890.24"
    );
    assert_eq!(v(batch, "size"), Value::Int(2.into()));
    conn.execute(
        "UPDATE btcpay_provenance_records SET fiat_currency='USD' WHERE id='b'",
        [],
    )
    .unwrap();
    let ctx = context(conn);
    let batch = get(&ctx, "payout_batch").unwrap();
    assert_eq!(v(batch, "mixed_currencies"), Value::Bool(true));
    assert_eq!(v(batch, "fiat_value_exact"), Value::Null);
    assert_eq!(v(batch, "fiat_currency"), Value::Null);
    conn.execute(
        "UPDATE btcpay_provenance_records SET fiat_currency=NULL WHERE id='b'",
        [],
    )
    .unwrap();
    let ctx = context(conn);
    assert_eq!(
        v(get(&ctx, "payout_batch").unwrap(), "fiat_value_exact"),
        Value::Null
    );
}

#[test]
fn commercial_stored_pages_and_context_keep_scope_filters_limits_and_ties() {
    let (_dir, book) = book();
    let conn = book.connection();
    for id in ["a", "b", "c"] {
        record(
            conn,
            id,
            "payment",
            Some("chain"),
            None,
            Some(9007199254740995),
            id,
        );
    }
    suggest(conn);
    for id in ["a", "b", "c"] {
        conn.execute("UPDATE commercial_links SET id=?,created_at='2025',reviewed_at=NULL WHERE btcpay_record_id=?",params![format!("link-{id}"),id]).unwrap();
    }
    conn.execute(
        "UPDATE commercial_links SET reviewed_at='2026',state='reviewed' WHERE id='link-a'",
        [],
    )
    .unwrap();
    let ctx = context(conn);
    let links = list(get(&ctx, "links").unwrap());
    assert_eq!(
        links.iter().map(|l| text(l, "id")).collect::<Vec<_>>(),
        ["link-a", "link-c", "link-b"]
    );
    let read = |command| commercial::execute(conn, &scope(), &command).unwrap();
    let all = read(commercial::Command::Links {
        state: Value::Null,
        limit: 0,
    });
    assert_eq!(
        list(&all).iter().map(|l| text(l, "id")).collect::<Vec<_>>(),
        ["link-c", "link-b", "link-a"]
    );
    assert_eq!(
        list(&read(commercial::Command::Links {
            state: s("REVIEWED"),
            limit: 100
        }))
        .len(),
        1
    );
    assert_eq!(
        list(&read(commercial::Command::Links {
            state: Value::Null,
            limit: -1
        }))
        .len(),
        1
    );
    let records = read(commercial::Command::Records {
        record_type: s("PAYMENT"),
        limit: 2,
    });
    assert_eq!(
        list(&records)
            .iter()
            .map(|r| text(r, "id"))
            .collect::<Vec<_>>(),
        ["c", "b"]
    );
    assert_eq!(
        v(&list(&records)[0], "amount"),
        Value::Float(90071.99254740994)
    );
    conn.execute(
        "UPDATE commercial_links SET profile_id='q' WHERE id='link-c'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE btcpay_provenance_records SET profile_id='q' WHERE id='c'",
        [],
    )
    .unwrap();
    assert_eq!(
        list(&read(commercial::Command::Records {
            record_type: Value::Null,
            limit: 100
        }))
        .len(),
        2
    );
    assert_eq!(list(get(&context(conn), "links").unwrap()).len(), 2);
    assert_eq!(
        list(&read(commercial::Command::Links {
            state: Value::Null,
            limit: 100
        }))
        .len(),
        2
    );
}

#[test]
fn document_transaction_matching_refuses_ambiguity_and_rolls_back_the_whole_suggestion_batch() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(
        conn,
        "payment",
        "payment",
        Some("chain"),
        None,
        Some(1001),
        "invoice",
    );
    record(conn, "invoice", "invoice", None, None, None, "invoice");
    create(conn, "Invoice", Some("invoice"));
    conn.execute_batch("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,occurred_at,kind,created_at) SELECT 'duplicate',workspace_id,profile_id,wallet_id,'duplicate',external_id,asset,direction,amount,occurred_at,kind,created_at FROM transactions;").unwrap();
    let result = suggest(conn);
    let links = list(get(&result, "suggestions").unwrap());
    assert_eq!(links.len(), 1);
    assert_eq!(text(&links[0], "link_type"), "document_btcpay");
    conn.execute("DELETE FROM transactions WHERE id='duplicate'", [])
        .unwrap();
    conn.execute("DELETE FROM commercial_links", []).unwrap();
    conn.execute(
        "UPDATE btcpay_provenance_records SET amount='corrupt' WHERE id='payment'",
        [],
    )
    .unwrap();
    assert_eq!(
        commercial::execute(conn, &scope(), &commercial::Command::Suggest { limit: 500 })
            .unwrap_err()
            .code,
        "internal_error"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM commercial_links", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    conn.execute(
        "UPDATE btcpay_provenance_records SET amount=1001 WHERE id='payment'",
        [],
    )
    .unwrap();
    conn.execute_batch("CREATE TRIGGER fail_suggestion BEFORE INSERT ON commercial_links WHEN NEW.link_type='document_btcpay' BEGIN SELECT RAISE(ABORT,'fail batch'); END;").unwrap();
    assert!(
        commercial::execute(conn, &scope(), &commercial::Command::Suggest { limit: 500 }).is_err()
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM commercial_links", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn review(
    conn: &Connection,
    link: &str,
    state: &str,
    kind: Option<&str>,
) -> Result<Value, kassiber_envelope::AppError> {
    commercial::execute(
        conn,
        &scope(),
        &commercial::Command::Review(commercial::Review {
            link: link.into(),
            state: s(state),
            reconciliation_state: Value::Null,
            commercial_kind: kind.map_or(Value::Null, s),
            notes: Value::Null,
        }),
    )
}
fn link_for(conn: &Connection, record: &str) -> String {
    conn.query_row(
        "SELECT id FROM commercial_links WHERE btcpay_record_id=?",
        [record],
        |r| r.get(0),
    )
    .unwrap()
}
fn tx_text(conn: &Connection, column: &str) -> Option<String> {
    conn.query_row(
        &format!("SELECT {column} FROM transactions WHERE id='tx'"),
        [],
        |r| r.get(0),
    )
    .unwrap()
}
fn version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT journal_input_version FROM profiles WHERE id='p'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn commercial_review_applies_exact_pricing_once_and_restores_the_snapshot() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(conn, "a", "payment", Some("chain"), None, Some(1001), "a");
    conn.execute("UPDATE btcpay_provenance_records SET origin_url=' https://shop.example/r/1 ', origin_kind='refund', pricing_timestamp=NULL, occurred_at='2025-01-01T00:00:00Z'", []).unwrap();
    suggest(conn);
    let link = link_for(conn, "a");
    let refused = review(conn, &link, "reviewed", Some("income")).unwrap_err();
    assert_eq!(
        refused.message,
        "Commercial income can only be applied to inbound transactions"
    );
    assert_eq!(
        review(conn, &link, "bogus", None).unwrap_err().message,
        "Unsupported link state 'bogus'"
    );
    assert_eq!(
        review(conn, "missing", "reviewed", None).unwrap_err().code,
        "not_found"
    );
    assert_eq!(
        tx_text(conn, "commercial_applied_link_id"),
        None,
        "refusals leave the transaction alone"
    );
    let before = version(conn);
    let reviewed = review(conn, &link, "reviewed", Some("expense")).unwrap();
    assert_eq!(v(&reviewed, "applied_to_transaction"), Value::Bool(true));
    assert_eq!(text(&reviewed, "reconciliation_state"), "matched");
    assert_eq!(text(&reviewed, "commercial_kind"), "expense");
    assert_eq!(version(conn), before + 1);
    for (column, expected) in [
        ("kind", "expense"),
        ("fiat_value_exact", "12.00"),
        ("fiat_rate_exact", "1198801198.8011988011988011988012"),
        ("fiat_currency", "EUR"),
        ("fiat_price_source", "import"),
        ("pricing_source_kind", "btcpay_payment"),
        ("pricing_provider", "btcpay"),
        ("pricing_pair", "BTC-EUR"),
        ("pricing_timestamp", "2025-01-01T00:00:00Z"),
        ("pricing_granularity", "invoice_payment"),
        ("pricing_method", "reviewed_commercial_link"),
        ("pricing_external_ref", "stable-a"),
        ("pricing_quality", "exact"),
        ("commercial_applied_link_id", link.as_str()),
    ] {
        assert_eq!(tx_text(conn, column).as_deref(), Some(expected), "{column}");
    }
    let attachments = || -> Vec<(String, String)> {
        let mut stmt = conn
            .prepare("SELECT label, source_url FROM attachments WHERE transaction_id='tx'")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        attachments(),
        [(
            "BTCPay refund".to_owned(),
            "https://shop.example/r/1".to_owned()
        )]
    );
    // Reviewing again keeps the first snapshot and the single URL reference.
    review(conn, &link, "reviewed", Some("transfer")).unwrap();
    assert_eq!(
        tx_text(conn, "kind").as_deref(),
        Some("withdrawal"),
        "transfer keeps the snapshot kind"
    );
    assert_eq!(attachments().len(), 1);
    let rejected = review(conn, &link, "rejected", None).unwrap();
    assert_eq!(v(&rejected, "restored_transaction"), Value::Bool(true));
    assert_eq!(text(&rejected, "reconciliation_state"), "ignored");
    for column in [
        "fiat_value_exact",
        "fiat_rate_exact",
        "pricing_source_kind",
        "commercial_applied_link_id",
        "fiat_currency",
    ] {
        assert_eq!(tx_text(conn, column), None, "{column}");
    }
    assert_eq!(tx_text(conn, "kind").as_deref(), Some("withdrawal"));
    let reopened = review(conn, &link, "suggested", None).unwrap();
    assert_eq!(v(&reopened, "restored_transaction"), Value::Bool(false));
}

/// `pricing_payload(amount_msat=tx["amount"], ...)` (f823b4729): the applied
/// prices are bounded, zeros canonical, and must imply a bounded valuation
/// for the transaction's own quantity, refused before anything is written.
#[test]
fn commercial_review_bounds_prices_for_the_transaction_amount() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(conn, "a", "payment", Some("chain"), None, Some(1001), "a");
    conn.execute(
        "UPDATE btcpay_provenance_records SET fiat_value_exact='1000000000'",
        [],
    )
    .unwrap();
    suggest(conn);
    let link = link_for(conn, "a");
    conn.execute("UPDATE transactions SET amount=1 WHERE id='tx'", [])
        .unwrap();
    let before = version(conn);
    let refused = review(conn, &link, "reviewed", Some("expense")).unwrap_err();
    assert_eq!(refused.code, "validation");
    assert_eq!(
        refused.message,
        "transaction pricing input: derived fiat_rate is outside the supported decimal range"
    );
    assert_eq!(tx_text(conn, "commercial_applied_link_id"), None);
    assert_eq!(tx_text(conn, "fiat_value_exact"), None);
    assert_eq!(version(conn), before);
    let state: String = conn
        .query_row(
            "SELECT state FROM commercial_links WHERE id=?",
            [&link],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "suggested");

    conn.execute("UPDATE transactions SET amount=1101 WHERE id='tx'", [])
        .unwrap();
    conn.execute(
        "UPDATE btcpay_provenance_records SET fiat_value_exact='0.00'",
        [],
    )
    .unwrap();
    review(conn, &link, "reviewed", Some("expense")).unwrap();
    assert_eq!(tx_text(conn, "fiat_value_exact").as_deref(), Some("0"));
    assert_eq!(tx_text(conn, "fiat_rate_exact").as_deref(), Some("0"));
}

#[test]
fn commercial_review_batches_payouts_and_refuses_income_and_custody_overwrites() {
    let (_dir, book) = book();
    let conn = book.connection();
    record(conn, "a", "payout", Some("chain"), None, Some(-1001), "a");
    record(conn, "b", "payout", Some("chain"), None, Some(2002), "b");
    conn.execute(
        "UPDATE btcpay_provenance_records SET fiat_value_exact='12.00'",
        [],
    )
    .unwrap();
    suggest(conn);
    let (a, b) = (link_for(conn, "a"), link_for(conn, "b"));
    assert_eq!(
        review(conn, &a, "reviewed", Some("income"))
            .unwrap_err()
            .message,
        "A BTCPay payout cannot be reviewed as income"
    );
    let reviewed = review(conn, &a, "reviewed", Some("refund")).unwrap();
    assert_eq!(
        v(&reviewed, "batch_link_ids"),
        Value::List(vec![s(b.as_str())])
    );
    assert_eq!(tx_text(conn, "fiat_value_exact").as_deref(), Some("24.00"));
    assert_eq!(
        tx_text(conn, "fiat_rate_exact").as_deref(),
        Some("799200799.20079920079920079920080")
    );
    assert_eq!(
        tx_text(conn, "pricing_granularity").as_deref(),
        Some("payout_batch")
    );
    let state = |link: &str| -> String {
        conn.query_row(
            "SELECT state FROM commercial_links WHERE id=?",
            [link],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(state(&b), "reviewed");
    let refused = review(conn, &b, "rejected", None).unwrap_err();
    assert_eq!(
        refused.message,
        "This payout was reviewed together with the other payouts of its transaction"
    );
    let reopened = review(conn, &b, "suggested", None).unwrap();
    assert_eq!(
        text(&reopened, "id"),
        a,
        "reopening a member acts on the batch owner"
    );
    assert_eq!(v(&reopened, "restored_transaction"), Value::Bool(true));
    assert_eq!(
        v(&reopened, "batch_link_ids"),
        Value::List(vec![s(b.as_str())])
    );
    assert_eq!(
        (state(&a), state(&b)),
        ("suggested".into(), "suggested".into())
    );
    assert_eq!(tx_text(conn, "fiat_value_exact"), None);
    conn.execute(
        "UPDATE btcpay_provenance_records SET fiat_currency='USD' WHERE id='b'",
        [],
    )
    .unwrap();
    assert_eq!(
        review(conn, &a, "reviewed", None).unwrap_err().message,
        "Payouts in one transaction use different currencies"
    );
    conn.execute(
        "UPDATE btcpay_provenance_records SET fiat_currency='EUR' WHERE id='b'",
        [],
    )
    .unwrap();
    conn.execute_batch("INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) VALUES('cc','cc','w','p',1,'transfer','active','2025'); INSERT INTO custody_component_legs(id,component_id,workspace_id,profile_id,ordinal,role,rail,asset,exposure,conservation_unit,amount_msat,transaction_id,created_at) VALUES('leg','cc','w','p',0,'source','bitcoin','BTC','btc','msat',1,'tx','2025');").unwrap();
    let refused = review(conn, &a, "reviewed", None).unwrap_err();
    assert_eq!(
        refused.message,
        "Commercial review cannot overwrite a transaction in an active custody review"
    );
    assert_eq!(state(&a), "suggested", "a refused review writes nothing");
}
