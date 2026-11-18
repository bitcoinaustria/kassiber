//! Bitcoin.de's BTC account statement CSV. The mapping is the API sync's
//! (`exchange_api/tests/bitcoinde_api.rs` checks both store the same rows);
//! these tests pin the file layout and its refusals. One test per rule.
use super::*;

/// A synthetic German statement (`;`, dot decimals, German wall time, newest
/// first) with the API fixture's bookings and a zero registration row.
pub(in crate::file_import) const BITCOINDE_CSV: &str = concat!(
    "Datum;Typ;Währung;Referenz;BTC-Adresse;Kurs;\"Einheit (Kurs)\";\"BTC vor Gebühr\";\"Menge vor Gebühr\";\"Einheit (Menge vor Gebühr)\";\"BTC nach Bitcoin.de-Gebühr\";\"Menge nach Bitcoin.de-Gebühr\";\"Einheit (Menge nach Bitcoin.de-Gebühr)\";\"Zu- / Abgang\";Kontostand\n",
    "\"2024-01-07 00:00:00\";Partnerprogramm;BTC;AFF-1;;;;;;;;;;0.00002000;0.00888000\n",
    "\"2024-01-06 00:00:00\";Verkauf;BTC;TRADE2;;42000.00;\"BTC / EUR\";0.00200000;84.00;EUR;0.00198000;83.58;EUR;-0.00200000;0.00886000\n",
    "\"2024-01-05 12:30:45\";\"Netzwerk-Gebühr\";BTC;5D2F0C8B1E4A3F6E7D9C0B1A2F3E4D5C6B7A8F9E0D1C2B3A4F5E6D7C8B9A0F1E;bc1qsyntheticpayoutaddress000000000000000;;;;;;;;;-0.00010000;0.01086000\n",
    "\"2024-01-05 12:30:45\";Auszahlung;BTC;5D2F0C8B1E4A3F6E7D9C0B1A2F3E4D5C6B7A8F9E0D1C2B3A4F5E6D7C8B9A0F1E;bc1qsyntheticpayoutaddress000000000000000;;;;;;;;;-0.00400000;0.01096000\n",
    "\"2024-01-04 00:00:00\";Kickback;BTC;KB-1;;;;;;;;;;0.00001000;0.01496000\n",
    "\"2024-01-03 00:00:00\";Einzahlung;BTC;a1545e842dd311d998cc4797be7502f573e3d1a6cb5c6cf0e8713e64e28be63a;bc1qsyntheticdepositaddress00000000000000;;;;;;;;;0.00500000;0.01495000\n",
    "\"2024-01-02 10:00:00\";Kauf;BTC;TRADE1;;40000.00;\"BTC / EUR\";0.01000000;400.00;EUR;0.00990000;398.00;EUR;0.00990000;0.00995000\n",
    "\"2023-12-01 09:00:00\";\"Welcome Btc\";BTC;;;;;;;;;;;0.00005000;0.00005000\n",
    "\"2023-11-30 08:00:00\";Registrierung;BTC;;;0.00;BTC;;;;;;;0.00000000;0.00000000\n",
);
const HEADER: &str = "Datum;Typ;Währung;Referenz;BTC-Adresse;Kurs;\"Einheit (Kurs)\";\"BTC vor Gebühr\";\"Menge vor Gebühr\";\"Einheit (Menge vor Gebühr)\";\"BTC nach Bitcoin.de-Gebühr\";\"Menge nach Bitcoin.de-Gebühr\";\"Einheit (Menge nach Bitcoin.de-Gebühr)\";\"Zu- / Abgang\";Kontostand\n";

/// `wallets import-bitcoinde --file` into the provider wallet.
pub(in crate::file_import) fn import_bitcoinde(conn: &Connection, content: &str) -> Result<Py, AppError> {
    import_into(conn, content, Format::Bitcoinde, "")
}
fn book() -> Connection {
    let conn = fixture();
    conn.execute_batch("UPDATE profiles SET fiat_currency='EUR';
        INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','BTC','fixed');").unwrap();
    conn
}
fn rows(conn: &Connection) -> Vec<(String, String, i64, Option<String>, String)> {
    let order = format!("SELECT t.external_id, t.kind, t.amount, t.fiat_value_exact, t.occurred_at FROM transactions t ORDER BY {}", kassiber_store::transaction_order::transaction_order_sql("t", false));
    conn.prepare(&order).unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).unwrap().collect::<Result<_, _>>().unwrap()
}

#[test]
fn bitcoinde_csv_imports_the_statement_in_full_mode_once() {
    let conn = book();
    let result = import_bitcoinde(&conn, BITCOINDE_CSV).unwrap();
    let field = |key| kassiber_envelope::get(&result, key).cloned();
    assert_eq!((field("imported"), field("bitcoinde_rows")), (Some(Py::from(7)), Some(Py::from(7))), "{result:?}");
    assert_eq!((field("input_format"), field("mode")), (Some(s("bitcoinde_csv")), Some(s("full"))));
    // 09:00 German winter time is 08:00 UTC.
    assert_eq!(rows(&conn)[0], ("bitcoinde:20231201T080000Z:0:5000000".into(), "income".into(), 5_000_000, None, "2023-12-01T08:00:00Z".into()));
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets WHERE label='Bitcoin.de' AND kind='bitcoinde' AND config_json='{\"source_format\": \"bitcoinde_csv\"}'"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions WHERE pricing_source_kind='exchange_execution' AND pricing_quality='exact' AND pricing_method='bitcoinde_trade' AND pricing_provider='Bitcoin.de'"), 2);
    let again = import_bitcoinde(&conn, BITCOINDE_CSV).unwrap();
    assert_eq!(kassiber_envelope::get(&again, "imported"), Some(&Py::from(0)));
    // An active custodial ledger imports in full only.
    let file = std::env::temp_dir().join(format!("kassiber-bitcoinde-mode-{}.csv", std::process::id()));
    std::fs::write(&file, BITCOINDE_CSV).unwrap();
    let request = Request { profile_id: "p".into(), wallet: String::new(), file: file.to_str().unwrap().into(), format: Format::Bitcoinde, mode: Some("relevant".into()), companion: None, timezone: None, changed_rows: None };
    let error = execute(&book(), request, None).unwrap_err();
    std::fs::remove_file(file).unwrap();
    assert_eq!(error.message, "bitcoinde_csv is an active custodial ledger import");
}

/// The English export, German summer time, the older Fidor express column
/// (the cash after the bank's fee) and same-second buys in balance order.
#[test]
fn bitcoinde_csv_reads_both_layouts_and_the_proven_order() {
    let conn = book();
    let content = concat!(
        "date;Booking type;currency;reference;BTC-Address;Rate;unit (rate);BTC before fee;amount before fee;unit (amount before fee);BTC excl. Bitcoin.de fee;amount after Bitcoin.de-fee;unit (amount after Bitcoin.de-fee);EUR excl. Bitcoin.de and Fidor fee;incoming / outgoing;balance;comment\n",
        "2024-07-01 12:00:00;Sale;BTC;TRADE9;;50000.00;BTC / EUR;0.10000000;5000.00;EUR;0.09900000;4975.00;EUR;;-0.10000000;0.19900000;\n",
        "2024-06-30 12:00:00;Purchase;BTC;TRADE8;;50000.00;BTC / EUR;0.20000000;10000.00;EUR;0.20000000;9950.00;EUR;;0.20000000;0.29900000;note\n",
        "2024-06-30 12:00:00;Purchase;BTC;EXPR1;;9000.00;BTC / EUR;0.10000000;900.00;EUR;0.09900000;895.50;EUR;895.95;0.09900000;0.09900000;\n",
    );
    import_bitcoinde(&conn, content).unwrap();
    assert_eq!(rows(&conn), [
        // 12:00 German summer time is 10:00 UTC; EXPR1 starts at zero, so it came first.
        ("bitcoinde:EXPR1".into(), "buy".into(), 9_900_000_000, Some("895.95".into()), "2024-06-30T10:00:00Z".into()),
        ("bitcoinde:TRADE8".into(), "buy".into(), 20_000_000_000, Some("9950.00".into()), "2024-06-30T10:00:00Z".into()),
        ("bitcoinde:TRADE9".into(), "sell".into(), 10_000_000_000, Some("4975.00".into()), "2024-07-01T10:00:00Z".into()),
    ]);
}

#[test]
fn bitcoinde_csv_refuses_what_it_cannot_map_exactly() {
    let row = |line: &str| format!("{HEADER}{line}\n");
    let cases = [
        (row("\"2024-01-04 00:00:00\";Kauf;BTC;T-1;;0.02;\"BTC / BCH\";0.01000000;400.00;EUR;0.00990000;398.00;EUR;0.00990000;0.00990000"), "is a trade whose market 'BTC / BCH' is not BTC / EUR"),
        (row("\"2024-01-04 00:00:00\";Kauf;BTC;T-1;;0.02;\"BTC / USDT\";0.01000000;400.00;USDT;0.00990000;398.00;USDT;0.00990000;0.00990000"), "trades BTC against USDT, which is not fiat"),
        (row("\"2024-01-04 00:00:00\";Kauf;BTC;T-1;;0.02;\"BTC / EUR\";0.01000000;400.00;EUR;0.00990000;398.00;EUR;0.01000000;0.01000000"), "moves a different BTC amount than its trade details"),
        // A statement filtered to buys leaves gaps in the running balance.
        (format!("{HEADER}\"2024-01-05 00:00:00\";Kauf;BTC;T-2;;0.02;\"BTC / EUR\";0.01000000;400.00;EUR;0.00990000;398.00;EUR;0.00990000;0.02000000\n\"2024-01-04 00:00:00\";Kauf;BTC;T-1;;0.02;\"BTC / EUR\";0.01000000;400.00;EUR;0.00990000;398.00;EUR;0.00990000;0.00990000\n"), "has a balance that does not follow the booking before it"),
        (row("\"2024-01-04 00:00:00\";Einzahlung;BTC;R-1;;;;;;;;;;0.10000000;0.30000000"), "starts from a balance of 20000000000 msat, not zero"),
        (row("\"2024-01-04 00:00:00\";Einzahlung;BTC;R-1;;;;;;;;;;0.100000000000000000000000000000001;0.100000000000000000000000000000001"), "has a BTC amount finer than a millisatoshi"),
        (row("\"2024-01-04 00:00:00\";buy_yubikey;BTC;Y-1;;;;;;;;;;-0.00100000;0.00000000"), "spends BTC in a Bitcoin.de shop"),
        (row("\"2024-01-04 00:00:00\";Umbuchung;BTC;U-1;;;;;;;;;;0.00100000;0.00100000"), "(type 'Umbuchung', reference 'U-1') has a type Kassiber does not map"),
        (row("\"2024-01-04 00:00:00\";\"Netzwerk-Gebühr\";BTC;R-1;;;;;;;;;;-0.00010000;-0.00010000"), "is a network fee without a payout in the same transaction"),
        (row("\"2024-01-04 00:00:00\";Einzahlung;BTC;R-1;;;;;;;;;;0,50000000;0,50000000"), "Kassiber reads only plain decimals with a decimal point"),
        (row("\"2024-10-27 02:30:00\";Einzahlung;BTC;R-1;;;;;;;;;;0.50000000;0.50000000"), "a wall time the summer-time change skips or repeats"),
        (row("\"2024-03-31 02:30:00\";Einzahlung;BTC;R-1;;;;;;;;;;0.50000000;0.50000000"), "a wall time the summer-time change skips or repeats"),
        (row("\"04.01.2024 00:00\";Einzahlung;BTC;R-1;;;;;;;;;;0.50000000;0.50000000"), "has the date '04.01.2024 00:00': a Date or Time Kassiber cannot read"),
        (row("\"2024-01-04 00:00:00\";Einzahlung;BTC;R-1;;;;;;;;;;0.50000000;0.50000000;extra"), "CSV line 2 has more fields than the heading"),
        ("Datum;Typ;Währung;Referenz;Notiz\n\"2024-01-04 00:00:00\";Einzahlung;BTC;R-1;x\n".to_owned(), "Bitcoin.de CSV has an unknown column 'Notiz'"),
        ("Datum;Typ;Typ\n1;2;3\n".to_owned(), "Bitcoin.de CSV has a repeated column 'Typ'"),
        ("Datum;Typ;Währung;Referenz\n\"2024-01-04 00:00:00\";Einzahlung;BTC;R-1\n".to_owned(), "Bitcoin.de CSV is missing required columns: btc_before, btc_after, fiat_after, fiat_after_unit, cashflow, balance"),
    ];
    for (content, message) in cases {
        let conn = book();
        let error = import_bitcoinde(&conn, &content).unwrap_err();
        assert_eq!(error.code, "validation", "{message}: {}", error.message);
        assert!(error.message.contains(message), "{message}: {}", error.message);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
    }
}

/// Kassiber reads only BTC from Bitcoin.de. In `bitcoin` scope another
/// coin's statement, or a row in another coin, is skipped and named in
/// `assets.next_step`; in `crypto` scope it is refused by the coin's name.
#[test]
fn bitcoinde_csv_names_other_coins_as_unread_in_either_scope() {
    let eth = "Datum;Typ;Währung;Referenz;ETH-Adresse;Kurs;\"Einheit (Kurs)\";\"ETH vor Gebühr\"\n\"2024-01-05 00:00:00\";Einzahlung;ETH;0x2;;;;\n\"2024-01-04 00:00:00\";Einzahlung;ETH;0x1;;;;\n";
    let mixed = format!("{BITCOINDE_CSV}\"2023-11-29 00:00:00\";Einzahlung;ETH;0x1;;;;;;;;;;1.00000000;1.00000000\n");
    let named = |scope: &str, rows: i64| format!(r#"{{"scope": "{scope}", "skipped": {{}}, "excluded": {{}}, "unread": {{"ETH": {rows}}}, "next_step": "Kassiber reads only BTC from Bitcoin.de, in either asset scope, and left out entries in ETH ({rows}); record them yourself (generic CSV/JSON import)"}}"#);
    for scope in ["bitcoin", "crypto"] {
        for (content, imported, rows) in [(eth.to_owned(), 0, 2), (mixed.clone(), 7, 1)] {
            let conn = book();
            if scope == "crypto" {
                conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('bd','ws','p','Bitcoin.de','bitcoinde','{\"asset_scope\": \"crypto\"}','2025-01-01')", []).unwrap();
            }
            let result = import_bitcoinde(&conn, &content).unwrap();
            let get = |key| kassiber_envelope::get(&result, key).cloned().unwrap();
            assert_eq!(get("imported"), Py::from(imported));
            assert_eq!(kassiber_pyjson::dumps(&get("assets"), &Default::default()).unwrap(), named(scope, rows));
        }
    }
}
