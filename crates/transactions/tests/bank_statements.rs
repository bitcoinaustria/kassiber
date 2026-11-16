//! Bank statements: one test per rule of docs/reference/bank-statements.md.
//! Synthetic names, IBANs (published example check digits) and amounts only.
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};
use kassiber_transactions::{
    bank_statements::{
        mask_ibans, parse,
        store::{self, ImportRequest, LineFilter},
        Format, Money, ParseOptions, ParsedStatement,
    },
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
    book.connection().execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025'); INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Main','EUR','2025'); INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('kraken','w','p','Kraken','kraken','2025'); INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,kind,fiat_currency,fiat_value_exact,created_at) VALUES('buy1','w','p','kraken','f1','BTC','inbound',2000000000,0,'2025-03-05T10:00:00Z','buy','EUR','1000.00','2025'),('buy2','w','p','kraken','f2','BTC','inbound',1000000000,0,'2025-03-20T10:00:00Z','buy','EUR',NULL,'2025');").unwrap();
    (dir, book)
}

fn text(v: &Value, key: &str) -> String {
    match get(v, key) {
        Some(Value::Str(t)) => t.as_str().unwrap().to_owned(),
        other => panic!("{key}: {other:?} in {v:?}"),
    }
}

fn int(v: &Value, key: &str) -> i64 {
    match get(v, key) {
        Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap(),
        other => panic!("{key}: {other:?}"),
    }
}

fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(v) => v,
        _ => panic!("{v:?}"),
    }
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn giro() -> ParseOptions {
    ParseOptions {
        account: Some("Test giro".into()),
        ..ParseOptions::default()
    }
}

fn mapped(map: Value) -> ParseOptions {
    ParseOptions {
        column_map: Some(map),
        ..giro()
    }
}

fn read(text: &str, options: &ParseOptions) -> ParsedStatement {
    parse(text.as_bytes(), options).unwrap_or_else(|e| panic!("{}", e.message))
}

fn refused(text: &str, options: &ParseOptions) -> String {
    parse(text.as_bytes(), options).unwrap_err().message
}

fn amounts(parsed: &ParsedStatement) -> Vec<String> {
    parsed.lines.iter().map(|l| l.amount.canonical()).collect()
}

const CAMT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02"><BkToCstmrStmt><Stmt><Id>STMT-2025-03</Id>
 <Acct><Id><IBAN>AT61 1904 3002 3457 3201</IBAN></Id><Ccy>EUR</Ccy><Svcr><FinInstnId><BIC>EXAMATWWXXX</BIC><Nm>Example Bank</Nm></FinInstnId></Svcr></Acct>
 <Ntry><Amt Ccy="EUR">1000.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts><BookgDt><Dt>2025-03-03</Dt></BookgDt><ValDt><Dt>2025-03-04</Dt></ValDt><AcctSvcrRef>REF-0001</AcctSvcrRef>
  <NtryDtls><TxDtls><Refs><EndToEndId>E2E-77</EndToEndId></Refs><RltdPties><Cdtr><Nm>Bitpanda GmbH</Nm></Cdtr><CdtrAcct><Id><IBAN>DE89370400440532013000</IBAN></Id></CdtrAcct></RltdPties>
  <RmtInf><Ustrd>Deposit ABC123</Ustrd><Ustrd>customer 42</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
 <Ntry><Amt Ccy="EUR">2500.5</Amt><CdtDbtInd>CRDT</CdtDbtInd><Sts><Cd>BOOK</Cd></Sts><BookgDt><DtTm>2025-03-05T09:00:00+01:00</DtTm></BookgDt>
  <NtryDtls><TxDtls><Refs><EndToEndId>NOTPROVIDED</EndToEndId></Refs><RltdPties><Dbtr><Pty><Nm>Example Employer AG</Nm></Pty></Dbtr></RltdPties>
  <RmtInf><Strd><CdtrRefInf><Ref>RF18539007547034</Ref></CdtrRefInf></Strd></RmtInf></TxDtls></NtryDtls></Ntry>
 <Ntry><Amt Ccy="EUR">5.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>PDNG</Sts><BookgDt><Dt>2025-03-06</Dt></BookgDt></Ntry>
</Stmt></BkToCstmrStmt></Document>"#;

#[test]
fn camt_reads_entries_parties_and_references_and_skips_pending() {
    let parsed = read(CAMT, &ParseOptions::default());
    assert_eq!(parsed.format, Some(Format::Camt053));
    assert_eq!(parsed.account.as_deref(), Some("AT611904300234573201"));
    assert_eq!(
        (parsed.bank_name.as_deref(), parsed.bank_bic.as_deref()),
        (Some("Example Bank"), Some("EXAMATWWXXX"))
    );
    assert_eq!(amounts(&parsed), ["-1000.00", "2500.50"]);
    let out = &parsed.lines[0];
    assert_eq!(
        (out.booking_date.as_str(), out.value_date.as_deref()),
        ("2025-03-03", Some("2025-03-04"))
    );
    assert_eq!(out.counterparty_name.as_deref(), Some("Bitpanda GmbH"));
    assert_eq!(
        out.counterparty_iban.as_deref(),
        Some("DE89370400440532013000")
    );
    assert_eq!(
        out.remittance.as_deref(),
        Some("Deposit ABC123 customer 42")
    );
    assert_eq!(
        (out.bank_reference.as_deref(), out.end_to_end_id.as_deref()),
        (Some("REF-0001"), Some("E2E-77"))
    );
    let incoming = &parsed.lines[1];
    assert_eq!(incoming.booking_date, "2025-03-05");
    assert_eq!(
        incoming.counterparty_name.as_deref(),
        Some("Example Employer AG")
    );
    assert_eq!(incoming.remittance.as_deref(), Some("RF18539007547034"));
    assert_eq!(incoming.end_to_end_id, None);
    assert_eq!(
        parsed.skipped,
        vec![(3, "entry status PDNG is not booked".to_owned())]
    );
}

#[test]
fn a_camt_batch_entry_is_one_line_never_split() {
    let batch = r#"<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.04"><BkToCstmrStmt><Stmt>
        <Ntry><Amt Ccy="EUR">300.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><BookgDt><Dt>2025-02-01</Dt></BookgDt><AcctSvcrRef>BATCH-9</AcctSvcrRef><NtryDtls>
         <TxDtls><Amt Ccy="EUR">100.00</Amt><RltdPties><Cdtr><Nm>Kraken</Nm></Cdtr></RltdPties><RmtInf><Ustrd>one</Ustrd></RmtInf></TxDtls>
         <TxDtls><Amt Ccy="EUR">200.00</Amt><RltdPties><Cdtr><Nm>Bitstamp</Nm></Cdtr></RltdPties><RmtInf><Ustrd>two</Ustrd></RmtInf></TxDtls>
        </NtryDtls></Ntry></Stmt></BkToCstmrStmt></Document>"#;
    let parsed = read(batch, &giro());
    assert_eq!(amounts(&parsed), ["-300.00"]);
    let line = &parsed.lines[0];
    assert_eq!(
        (
            line.counterparty_name.as_deref(),
            line.remittance.as_deref()
        ),
        (None, Some("one two"))
    );
    assert_eq!(line.bank_reference.as_deref(), Some("BATCH-9"));
}

#[test]
fn camt_refuses_other_messages_dtds_and_malformed_entries_without_quoting() {
    let doc = |ns: &str| {
        format!(
            r#"<Document xmlns="{ns}"><BkToCstmrStmt><Stmt><Ntry/></Stmt></BkToCstmrStmt></Document>"#
        )
    };
    for ns in [
        "urn:iso:std:iso:20022:tech:xsd:camt.052.001.02",
        "urn:iso:std:iso:20022:tech:xsd:camt.053.001.09",
        "urn:x:AT611904300234573201",
    ] {
        let message = refused(&doc(ns), &ParseOptions::default());
        assert!(
            message.contains("camt.053.001.02 to .08") && !message.contains(ns),
            "{message}"
        );
    }
    let dtd = r#"<?xml version="1.0"?><!DOCTYPE Document [<!ENTITY x "y">]><Document/>"#;
    assert!(refused(dtd, &ParseOptions::default()).contains("not readable XML"));
    assert!(refused(
        &CAMT.replace("<CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK", "<Sts>BOOK"),
        &ParseOptions::default()
    )
    .contains("CdtDbtInd"));
    let float = refused(&CAMT.replace("1000.00", "1e3"), &ParseOptions::default());
    assert!(
        float.contains("not a plain decimal") && !float.contains("1e3"),
        "{float}"
    );
}

#[test]
fn mt940_is_refused_by_name() {
    let mt940 = ":20:STARTUMS\n:25:AT611904300234573201\n:60F:C241230EUR0,00\n:61:2412311231DR1000,00NTRFNONREF\n-";
    let error = parse(mt940.as_bytes(), &giro()).unwrap_err();
    assert_eq!(
        get(error.details.as_ref().unwrap(), "reason"),
        Some(&s("bank_statement_mt940"))
    );
    let chosen = Format::choose(Some("mt940")).unwrap_err();
    assert_eq!(
        get(chosen.details.as_ref().unwrap(), "reason"),
        Some(&s("bank_statement_mt940"))
    );
}

fn latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap())
        .collect()
}

#[test]
fn csv_finds_the_header_below_a_preamble_in_latin1_with_comma_decimals() {
    let text = "Kontoinhaber;Example Person\n\nBuchungsdatum;Valuta;Empfänger;IBAN;Verwendungszweck;Betrag;Währung\n\
03.03.2025;04.03.2025;Bitpanda GmbH;AT61 1904 3002 3457 3201;Einzahlung;-1.000,00;EUR\n\
05.03.2025;05.03.2025;\"Müller; Example\";;Gehalt;2.500,50;EUR\n;;;;Neuer Kontostand;1.500,50;EUR\n";
    let parsed = parse(&latin1(text), &giro()).unwrap();
    assert_eq!(amounts(&parsed), ["-1000.00", "2500.50"]);
    assert_eq!(
        parsed.lines[0].counterparty_iban.as_deref(),
        Some("AT611904300234573201")
    );
    assert_eq!(
        parsed.lines[1].counterparty_name.as_deref(),
        Some("Müller; Example")
    );
    assert_eq!((parsed.lines[0].position, parsed.skipped.len()), (4, 1));
}

/// The options a table row names: the stated account plus one setting.
fn options(name: &str) -> ParseOptions {
    let currency = |c: &str| ParseOptions {
        currency: Some(c.into()),
        ..giro()
    };
    let amount_column = |amount: Value| {
        mapped(dict([
            ("booking_date", s("Date")),
            ("amount", amount),
            ("currency", s("Currency")),
        ]))
    };
    match name {
        "eur" | "usd" | "eth" => currency(&name.to_uppercase()),
        "comma" => mapped(dict([("decimal", s(","))])),
        "mdy" => mapped(dict([("date_format", s("MM/DD/YYYY"))])),
        "third" => amount_column(Value::Int(3.into())),
        "by name" => amount_column(s("Amount")),
        "custom" => mapped(dict([
            ("booking_date", s("Tag")),
            ("amount", s("Wert")),
            ("counterparty", s("Gegenseite")),
            ("remittance", Value::List(vec![s("Zweck 1"), s("Zweck 2")])),
            ("currency", s("Waehrungseinheit")),
        ])),
        "missing" => mapped(dict([("booking_date", s("Tag")), ("amount", s("Missing"))])),
        "unknown" => mapped(dict([("wert", s("Wert"))])),
        "nobody" => ParseOptions::default(),
        _ => giro(),
    }
}

const CUSTOM: &str = "Info\nTag;Wert;Gegenseite;Zweck 1;Zweck 2;Waehrungseinheit\n03.03.2025;-1.000,00;Relai AG;Kauf;BTC;EUR\n";

#[test]
fn csv_reads_every_supported_layout_exactly() {
    #[rustfmt::skip]
    let cases: &[(&str, &str, &[&str], &str)] = &[
        ("Booking date,Debit,Credit\n2025-03-03,\"1,000.00\",\n2025-03-07,,12.5\n", "eur", &["-1000.00", "12.50"], "EUR"),
        ("Buchungstag;Umsatz;Soll/Haben;WKZ\n01.02.2025;100,00;S;EUR\n02.02.2025;5,00;H;EUR\n03.02.2025;7,25-;;EUR\n", "giro", &["-100.00", "5.00", "-7.25"], "EUR"),
        ("Datum;Betrag;Währung\n01.02.2025;1.000;EUR\n", "comma", &["1000.00"], "EUR"),
        ("Datum;Betrag;Währung\n01.02.2025;1.000;EUR\n02.02.2025;2,50;EUR\n", "giro", &["1000.00", "2.50"], "EUR"),
        ("Date;Amount;Amount;Currency\n2025-01-01;-1,00;-2,00;EUR\n", "third", &["-2.00"], "EUR"),
        ("Date;Amount;Currency\n2025-01-01;-1.234.567,89;EUR\n2025-01-02;12,50;EUR\n", "giro", &["-1234567.89", "12.50"], "EUR"),
        ("Date,Amount,Currency\n2025-01-01,\"1,234,567.89\",EUR\n", "giro", &["1234567.89"], "EUR"),
        ("Date,Amount (CHF),Payee\n2025-01-02,-50.00,River\n", "giro", &["-50.00"], "CHF"),
        ("Datum;Betrag\n02.01.2025;-50,00 EUR\n", "giro", &["-50.00"], "EUR"),
        (CUSTOM, "custom", &["-1000.00"], "EUR"),
    ];
    for (text, option, expected, currency) in cases {
        let parsed = read(text, &options(option));
        assert_eq!(
            (amounts(&parsed), parsed.lines[0].currency.as_str()),
            (
                expected.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
                *currency
            ),
            "{text}"
        );
    }
    let custom = &read(CUSTOM, &options("custom")).lines[0];
    assert_eq!(
        (
            custom.counterparty_name.as_deref(),
            custom.remittance.as_deref()
        ),
        (Some("Relai AG"), Some("Kauf BTC"))
    );
    let dates = "Date,Amount,Currency\n03/04/2025,-10.00,EUR\n25/06/2025,-11.00,EUR\n";
    assert_eq!(
        read(dates, &giro()).lines[0].booking_date,
        "2025-04-03",
        "a day above 12 decides the order"
    );
    let us = "Date,Amount,Currency\n03/04/2025,-10.00,EUR\n05/06/2025,-11.00,EUR\n";
    assert_eq!(
        read(us, &options("mdy")).lines[0].booking_date,
        "2025-03-04"
    );
}

#[test]
fn csv_refuses_what_it_cannot_read_by_name() {
    #[rustfmt::skip]
    let cases: &[(&str, &str, &str)] = &[
        ("Datum;Betrag;Währung\n01.02.2025;1.000;EUR\n", "giro", "thousands separator or a decimal mark"),
        ("Date,Amount,Currency\n03/04/2025,-10.00,EUR\n05/06/2025,-11.00,EUR\n", "giro", "day-first or month-first"),
        ("Date,Amount,Currency\n2025-01-01,1.00,EUR\n2025-02-30,1.00,EUR\n", "giro", "invalid dates"),
        ("Date,Debit,Credit,Currency\n2025-01-01,1.00,2.00,EUR\n", "giro", "both debit and credit carry"),
        ("Date;Amount;Amount;Currency\n2025-01-01;-1,00;-2,00;EUR\n", "giro", "Several columns could be amount: columns 2, 3"),
        ("Datum;Betrag;Umsatz;Währung\n01.01.2025;-1,00;-2,00;EUR\n", "giro", "columns 2, 3"),
        ("Date;Amount;Amount;Currency\n2025-01-01;-1,00;-2,00;EUR\n", "by name", "appears 2 times"),
        (CUSTOM, "missing", "every column the map names"),
        (CUSTOM, "unknown", "Unknown column-map field"),
        ("Foo;Bar\n1;2\n", "giro", "not recognized as a bank statement"),
        ("Date;Amount;Currency\n2025-01-01;-100.00 USD;EUR\n", "giro", "currency column says EUR but the amount says USD"),
        ("Date;Amount (CHF);Currency\n2025-01-01;-1.00;EUR\n", "giro", "the amount header says CHF"),
        ("Date;Amount;Currency\n2025-01-01;-1.00;EUR\n", "usd", "the --currency says USD"),
        ("Date;Amount\n2025-01-01;-1.00\n", "giro", "does not state a currency"),
        ("Date;Amount;Currency\n2025-01-01;-1.00;EUR\n2025-01-02;-1.00;USDT\n", "giro", "Row 3: the currency is not a supported fiat currency"),
        ("Date;Amount\n2025-01-01;-1.00 BTC\n", "giro", "Row 2: the currency is not"),
        ("Date;Amount (XBT)\n2025-01-01;-1.00\n", "giro", "The amount header: the currency is not"),
        ("Date;Amount\n2025-01-01;-1.00\n", "eth", "--currency: the currency (ETH)"),
        ("Date,Amount,Currency\n2025-01-01,\"-1,2,3\",EUR\n", "giro", "not an amount"),
        ("Date,Amount,Currency\n2025-01-01,\"1234,567.00\",EUR\n", "giro", "not an amount"),
        ("Date,Amount,Currency\n2025-01-01,€100.00,EUR\n", "giro", "not an amount"),
        ("Date,Amount,Currency\n2025-01-01,\"1 234,50\",EUR\n", "giro", "not an amount"),
        ("Date,Amount,Currency\n2025-01-01,(100.00),EUR\n", "giro", "not an amount"),
        ("Date;Amount;Currency\n2025-01-01;-1.00;EUR\n", "nobody", "does not name the account"),
    ];
    for (text, option, expected) in cases {
        let message = refused(text, &options(option));
        assert!(message.contains(expected), "{text}\n{message}");
    }
    let other = ParseOptions {
        account: Some("DE89370400440532013000".into()),
        ..ParseOptions::default()
    };
    assert!(refused(CAMT, &other).contains("different account"));
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("Date;Amount\n".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let error = parse(&utf16, &ParseOptions::default()).unwrap_err();
    assert!(error.message.contains("UTF-16"), "{}", error.message);
    let btc = CAMT.replace(r#"<Amt Ccy="EUR">1000.00"#, r#"<Amt Ccy="BTC">1000.00"#);
    let error = parse(btc.as_bytes(), &ParseOptions::default()).unwrap_err();
    assert!(
        error.message.contains("Entry 1: the currency is not"),
        "{}",
        error.message
    );
    assert_eq!(
        get(error.details.as_ref().unwrap(), "reason"),
        Some(&s("bank_statement_currency"))
    );
}

#[test]
fn refusals_never_quote_the_file() {
    let secret = "AT611904300234573201";
    for (text, options) in [
        (
            format!("Date;Amount;Currency\n2025-01-01;{secret};EUR\n"),
            giro(),
        ),
        (
            format!("Date;Amount;Currency;Soll/Haben\n2025-01-01;-1.00;EUR;{secret}\n"),
            giro(),
        ),
        (CAMT.replace("1000.00", secret), ParseOptions::default()),
    ] {
        let error = parse(text.as_bytes(), &options).unwrap_err();
        let shown = format!("{} {:?} {:?}", error.message, error.hint, error.details);
        assert!(!shown.contains("1904300234"), "{shown}");
    }
}

#[test]
fn free_text_shows_every_valid_iban_masked_and_nothing_else() {
    assert_eq!(
        mask_ibans("from AT61 1904 3002 3457 3201 for 100,00"),
        "from AT** **** **** **** 3201 for 100,00"
    );
    assert_eq!(
        mask_ibans("DE89370400440532013000/x"),
        "DE** **** **** **** **30 00/x"
    );
    assert_eq!(
        mask_ibans("AT611904300234573202 order AB12 3456"),
        "AT611904300234573202 order AB12 3456",
        "bad check digits and short codes stay"
    );
}

#[test]
fn money_is_exact_decimal_text() {
    let m = |v: &str| Money::parse(v).unwrap();
    assert_eq!(
        (
            m("2500.5").canonical(),
            m("-0.000100").canonical(),
            m("7").canonical()
        ),
        ("2500.50".into(), "-0.0001".into(), "7.00".into())
    );
    assert!(m("1.50") == m("1.5") && m("-2") < m("1"));
    assert_eq!(
        m("50000").times(m("0.00500000000")).unwrap().canonical(),
        "250.00"
    );
    for bad in ["1e3", "1,5", " 1", "", "-", "1.2.3"] {
        assert!(Money::parse(bad).is_none(), "{bad}");
    }
}

// Storage.

fn import_file(
    conn: &Connection,
    root: &Path,
    file: &Path,
    options: ParseOptions,
    dry_run: bool,
) -> kassiber_transactions::bank_statements::Result<Value> {
    let request = ImportRequest {
        file: file.to_string_lossy().into_owned(),
        options,
        label: None,
        dry_run,
    };
    store::import_statement(conn, &scope(), root, &request)
}

fn own() -> ParseOptions {
    ParseOptions {
        account: Some("AT611904300234573201".into()),
        ..ParseOptions::default()
    }
}

fn csv_ref(lines: &[&str]) -> String {
    let mut out =
        String::from("Buchungsdatum;Empfänger;IBAN;Verwendungszweck;Bankreferenz;Betrag;Währung\n");
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

const MARCH: &str =
    "03.03.2025;Payward Ltd;AT61 1904 3002 3457 3201;Kraken deposit;REF-MARCH;-1.000,00;EUR";
const SALARY: &str = "05.03.2025;Example Employer AG;;Gehalt;REF-SALARY;2.500,00;EUR";
const APRIL: &str = "02.04.2025;Example Shop;;Einkauf;REF-APRIL;-20,00;EUR";

#[test]
fn imports_dedupe_by_reference_and_roll_back_exactly_what_they_added() {
    let (dir, book) = book();
    let conn = book.connection();
    let root = dir.path().join("attachments");
    let first = dir.path().join("statement_AT611904300234573201.csv");
    fs::write(&first, csv_ref(&[MARCH, SALARY])).unwrap();
    let preview = import_file(conn, &root, &first, own(), true).unwrap();
    assert_eq!(
        (
            int(&preview, "lines_new"),
            store::tables_present(conn).unwrap()
        ),
        (2, false),
        "a dry run creates no tables"
    );
    let imported = import_file(conn, &root, &first, own(), false).unwrap();
    assert_eq!(
        text(get(&imported, "document").unwrap(), "document_type"),
        "statement"
    );
    let import_id = text(get(&imported, "import").unwrap(), "id");
    let again = import_file(conn, &root, &first, own(), false).unwrap();
    assert_eq!(
        (
            get(&again, "already_imported"),
            int(&again, "lines_existing")
        ),
        (Some(&Value::Bool(true)), 2)
    );
    let second = dir.path().join("april.csv");
    fs::write(&second, csv_ref(&[SALARY, APRIL])).unwrap();
    let overlap = import_file(conn, &root, &second, own(), false).unwrap();
    assert_eq!(
        (int(&overlap, "lines_new"), int(&overlap, "lines_existing")),
        (1, 1)
    );
    // The same bytes for another own account are another statement.
    let other = ParseOptions {
        account: Some("DE89370400440532013000".into()),
        ..ParseOptions::default()
    };
    assert_eq!(
        int(
            &import_file(conn, &root, &first, other, false).unwrap(),
            "lines_new"
        ),
        2
    );
    // No full counterparty IBAN, no IBAN hash and no original file name is stored.
    let dump: String = conn.query_row("SELECT group_concat(coalesce(counterparty_iban_masked,'')||coalesce(remittance,''),'|') FROM bank_statement_lines", [], |r| r.get(0)).unwrap();
    let names: String = conn
        .query_row(
            "SELECT group_concat(original_filename||label,'|') FROM attachments",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !dump.contains("1904300234")
            && !dump.contains("1904 3002")
            && !names.contains("1904300234"),
        "{dump} {names}"
    );
    let lines = store::list_lines(
        conn,
        &scope(),
        &LineFilter {
            direction: "out".into(),
            document: None,
            limit: 0,
        },
    )
    .unwrap();
    assert_eq!(list(&lines).len(), 3);
    // Rollback plans first, then deletes its lines, document and file.
    let stored: String = conn.query_row("SELECT a.stored_relpath FROM attachments a JOIN bank_statement_imports i ON i.attachment_id=a.id WHERE i.id=?", [&import_id], |r| r.get(0)).unwrap();
    assert_eq!(
        int(
            &store::rollback(conn, &scope(), &root, &import_id, false).unwrap(),
            "lines_to_delete"
        ),
        2
    );
    store::rollback(conn, &scope(), &root, &import_id, true).unwrap();
    assert_eq!(count(conn, "SELECT COUNT(*) FROM bank_statement_lines"), 3);
    assert!(!root.join(stored).exists());
    assert_eq!(
        store::rollback(conn, &scope(), &root, &import_id, true)
            .unwrap_err()
            .code,
        "not_found"
    );
}

#[test]
fn lines_kassiber_cannot_place_are_refused_by_name() {
    let (dir, book) = book();
    let conn = book.connection();
    let root = dir.path().join("attachments");
    let plain = |name: &str, body: &str| {
        let file = dir.path().join(name);
        fs::write(
            &file,
            format!("Buchungsdatum;Empfänger;Verwendungszweck;Betrag;Währung\n{body}"),
        )
        .unwrap();
        file
    };
    let reason = |e: kassiber_envelope::AppError| text(e.details.as_ref().unwrap(), "reason");
    import_file(
        conn,
        &root,
        &plain("a.csv", "03.03.2025;Shop;x;-10,00;EUR\n"),
        own(),
        false,
    )
    .unwrap();
    // The same transfer without a bank reference, in another file: refused.
    let same = plain(
        "b.csv",
        "03.03.2025;Shop;x;-10,00;EUR\n04.03.2025;Shop;y;-1,00;EUR\n",
    );
    assert_eq!(
        reason(import_file(conn, &root, &same, own(), false).unwrap_err()),
        "bank_statement_line_ambiguous"
    );
    // Same account, date, amount and (no) reference but other details: refused.
    let differs = plain("c.csv", "03.03.2025;Shop;z;-10,00;EUR\n");
    let error = import_file(conn, &root, &differs, own(), false).unwrap_err();
    assert_eq!(
        (
            reason(error.clone()),
            text(error.details.as_ref().unwrap(), "field")
        ),
        (
            "bank_statement_line_conflict".into(),
            "remittance text".into()
        )
    );
    // The same bytes read with another column map: refused naming the import.
    let remapped = ParseOptions {
        column_map: Some(dict([
            ("booking_date", s("Buchungsdatum")),
            ("amount", s("Betrag")),
            ("currency", s("Währung")),
        ])),
        ..own()
    };
    assert_eq!(
        reason(
            import_file(
                conn,
                &root,
                &plain("a.csv", "03.03.2025;Shop;x;-10,00;EUR\n"),
                remapped,
                false
            )
            .unwrap_err()
        ),
        "bank_statement_reparsed"
    );
    assert_eq!(
        count(conn, "SELECT COUNT(*) FROM bank_statement_imports"),
        1
    );
    assert_eq!(
        count(conn, "SELECT COUNT(*) FROM attachments"),
        1,
        "refused imports leave nothing behind"
    );
}
