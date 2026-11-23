//! Binance supplemental, Ledger Live and Coinfinity CSVs under a wallet's
//! asset scope (docs/plan/24-crypto-assets.md): the `bitcoin` scope counts
//! the rows it leaves out and reports them, the `crypto` scope keeps them,
//! and a row Kassiber cannot book is refused by its row number.
use super::*;

/// Imports `content` as `format` into the 'Source' wallet in `scope`.
fn run(format: Format, scope: &str, content: &str) -> Result<Py, AppError> {
    let conn = fixture();
    conn.execute_batch("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'")
        .unwrap();
    assets::Scope::parse(scope)
        .unwrap()
        .store(&conn, "p", "wallet")
        .unwrap();
    let file = std::env::temp_dir().join(format!(
        "kassiber-supplemental-{}-{:?}.csv",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&file, content).unwrap();
    let mode = matches!(format, Format::Binance | Format::Coinfinity).then(|| "full".into());
    let result = execute(
        &conn,
        Request {
            profile_id: "p".into(),
            wallet: "Source".into(),
            file: file.to_str().unwrap().into(),
            format,
            mode,
            companion: None,
            timezone: None,
        },
        None,
    )
    .map(|result| {
        let rows = scalar(&conn, "SELECT count(*) FROM transactions WHERE wallet_id='wallet'");
        let assets: Vec<String> = conn
            .prepare("SELECT asset||' '||kind||' '||amount||' fee='||fee FROM transactions WHERE wallet_id='wallet' ORDER BY occurred_at")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows as usize, assets.len());
        let mut result = result;
        kassiber_envelope::set(
            &mut result,
            "stored",
            Py::List(assets.into_iter().map(s).collect()),
        );
        result
    });
    std::fs::remove_file(file).unwrap();
    result
}

fn field(result: &Py, key: &str) -> String {
    kassiber_pyjson::dumps(
        kassiber_envelope::get(result, key).unwrap_or(&Py::Null),
        &Default::default(),
    )
    .unwrap()
}

const SKIPPED: &str = r#"{"scope": "bitcoin", "skipped": {"ETH": 1}, "excluded": {}, "next_step": "kassiber wallets update --wallet 'Source' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep ETH"}"#;

const AUTO: &str = "timestamp UTC,base asset symbol,quote asset amount + symbol,trading fee (in quote asset),base asset amount + symbol,source of funds\n";
const DIVIDENDS: &str = "id,amount,asset,divTime,enInfo\n";
const LEDGER: &str = "Operation Date,Currency Ticker,Operation Type,Operation Amount,Operation Fees,Operation Hash,Account Name\n";
const COINFINITY: &str = "Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\n";

#[test]
fn supplemental_csvs_count_other_assets_in_the_bitcoin_scope_and_keep_them_in_crypto() {
    let cases = [
        (
            Format::Binance,
            format!("{AUTO}2025-01-02 03:04:05,BTC,100.00 EUR,1.00 EUR,0.002 BTC,Spot\n2025-01-03 03:04:05,ETH,100.00 EUR,1.00 EUR,0.05 ETH,Spot\n"),
            r#"["BTC buy 200000000 fee=0", "ETH buy 5000000000 fee=0"]"#,
        ),
        (
            Format::Binance,
            format!("{DIVIDENDS}d1,0.0001,BTC,1735787045000,Launchpool\nd2,0.5,ETH,1735873445000,Launchpool\n"),
            r#"["BTC income 10000000 fee=0", "ETH income 50000000000 fee=0"]"#,
        ),
        (
            Format::LedgerLive,
            format!("{LEDGER}2025-01-02T01:02:03Z,BTC,IN,0.002,0,h1,Bitcoin\n2025-01-03T01:02:03Z,ETH,IN,0.5,0.001,h2,Ethereum\n"),
            r#"["BTC deposit 200000000 fee=0", "ETH deposit 50000000000 fee=0"]"#,
        ),
        (
            Format::Coinfinity,
            format!("{COINFINITY}o1,sell,2025-01-02,100,0.002,BTC,,0,1,t1\no2,sell,2025-01-03,100,0.05,ETH,,0,1,t2\n"),
            r#"["BTC buy 200000000 fee=0", "ETH buy 5000000000 fee=0"]"#,
        ),
    ];
    for (format, content, crypto) in cases {
        let kept = run(format, "bitcoin", &content).unwrap();
        assert_eq!(field(&kept, "assets"), SKIPPED, "{format:?}");
        assert_eq!(
            field(&kept, "stored"),
            format!("[{}]", &crypto[1..crypto.find(',').unwrap()]),
            "{format:?}"
        );
        let kept = run(format, "crypto", &content).unwrap();
        assert_eq!(field(&kept, "stored"), crypto, "{format:?}");
        assert_eq!(
            field(&kept, "assets"),
            r#"{"scope": "crypto", "skipped": {}, "excluded": {}}"#
        );
    }
}

/// A Coinfinity row in LBTC is Bitcoin (plan 24's Bitcoin family is BTC and
/// LBTC): a wallet import keeps it in either scope and counts nothing as
/// skipped. The reference read only BTC and left such a row out; the
/// recorded session that pinned that reading is dropped
/// (tests/data/sessions/dropped.tsv).
#[test]
fn coinfinity_keeps_an_lbtc_row_as_bitcoin_in_either_scope() {
    // The same order in BTC and in LBTC books the same way.
    let content = format!(
        "{COINFINITY}o1,sell,2025-01-02,100,0.002,BTC,,0.00001,1.20,t1\no2,sell,2025-01-03,100,0.002,LBTC,,0.00001,1.20,t2\n"
    );
    for scope in ["bitcoin", "crypto"] {
        let kept = run(Format::Coinfinity, scope, &content).unwrap();
        assert_eq!(
            field(&kept, "stored"),
            r#"["BTC buy 200000000 fee=0", "LBTC buy 200000000 fee=0"]"#,
            "{scope}"
        );
        assert_eq!(
            field(&kept, "assets"),
            format!(r#"{{"scope": "{scope}", "skipped": {{}}, "excluded": {{}}}}"#),
            "{scope}"
        );
    }
}

#[test]
fn supplemental_csvs_refuse_rows_they_cannot_book_by_row() {
    for (format, scope, content, message) in [
        (
            Format::Binance,
            "bitcoin",
            format!("{AUTO}2025-01-02 03:04:05,BTC,100.00 EUR,1.00 EUR,0.002 BTC,Spot\n2025-01-03 03:04:05,BTC,100.00 USDT,1.00 USDT,0.002 BTC,Spot\n"),
            "Binance supplemental row 2 buys BTC with USDT, not fiat",
        ),
        (
            Format::Binance,
            "crypto",
            format!("{AUTO}2025-01-03 03:04:05,BTC,100.00 EUR,0.01 BNB,0.002 BTC,Spot\n"),
            "Binance supplemental row 1 pays its fee in BNB, not in EUR",
        ),
        (
            Format::Binance,
            "crypto",
            format!("{DIVIDENDS}d1,5,EUR,1735787045000,Cashback\n"),
            "Binance supplemental row 1 has asset 'EUR', which is not a crypto asset",
        ),
        // Earlier builds left these two out of a bitcoin-scope import.
        (
            Format::Binance,
            "bitcoin",
            format!("{DIVIDENDS}d1,5,EUR,1735787045000,Cashback\n"),
            "Binance supplemental row 1 has asset 'EUR', which is not a crypto asset",
        ),
        (
            Format::Binance,
            "bitcoin",
            format!("{AUTO}2025-01-03 03:04:05,ETH,100.00 EUR,1.00 EUR,10 SOL,Spot\n"),
            "Binance supplemental row 1 names base asset ETH but an amount in SOL",
        ),
        (
            Format::LedgerLive,
            "crypto",
            format!("{LEDGER}2025-01-03T01:02:03Z,USDT,OUT,-10,0.001,h2,Tether\n"),
            "Ledger Live row 1 sends USDT with a fee of 0.001",
        ),
        (
            Format::LedgerLive,
            "crypto",
            format!("{LEDGER}2025-01-02T01:02:03Z,BTC,OUT,0,0.0001,h1,Bitcoin\n2025-01-03T01:02:03Z,ETH,OUT,0,0.004,h2,Ethereum\n"),
            "Ledger Live row 2 has no ETH amount but a fee of 0.004",
        ),
        (
            Format::Coinfinity,
            "bitcoin",
            format!("{COINFINITY}o1,sell,2025-01-02,100,0.002,BTC,,0,1,t1\no2,refund,2025-01-03,100,0.002,BTC,,0,1,t2\n"),
            "Coinfinity row 2 has unsupported Type 'refund'",
        ),
    ] {
        let error = run(format, scope, &content).unwrap_err();
        assert_eq!(error.message, message, "{format:?}");
    }
}

/// Generic CSV/JSON and BTCPay rows name their own asset, with no tally:
/// the coordinator refuses a fiat code (`EUR`) or a Bitcoin unit in any
/// scope and another crypto ticker (`ETH`, or `AAPL`, which a ticker alone
/// cannot tell from a coin) in the `bitcoin` scope, naming the row. BTC,
/// LBTC and Liquid asset ids import as before.
#[test]
fn untallied_formats_refuse_assets_their_wallet_does_not_hold() {
    let liquid = "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d";
    let csv = |asset: &str| {
        format!("date,txid,direction,asset,amount,fee\n2025-01-02,a,in,BTC,1,0\n2025-01-03,b,in,{asset},1,0\n")
    };
    let btcpay = |currency: &str| {
        json!([
            {"TransactionId":"t1","Timestamp":"2025-01-02","Amount":"0.001","Currency":"BTC"},
            {"TransactionId":"t2","Timestamp":"2025-01-03","Amount":"0.001","Currency":currency}
        ])
        .to_string()
    };
    let keep = r#"kassiber wallets update --wallet 'Source' --config '{"asset_scope": "crypto"}'"#;
    let scoped = |row: &str, code: &str| {
        (
            format!(
                "{row} has asset '{code}', which this wallet's bitcoin asset scope does not import"
            ),
            Some(format!(
                "To import {code}, run {keep}, then import the file again."
            )),
        )
    };
    let fiat = |row: &str, code: &str| {
        (
            format!("{row} has asset '{code}', which is not a crypto asset"),
            Some("Kassiber imports BTC and, in a wallet whose asset_scope is crypto, other crypto assets; check the row in the source.".to_owned()),
        )
    };
    for (format, scope, content, refusal) in [
        (
            Format::Csv,
            "bitcoin",
            csv("ETH"),
            Some(scoped("csv row 2", "ETH")),
        ),
        (
            Format::Csv,
            "bitcoin",
            csv("AAPL"),
            Some(scoped("csv row 2", "AAPL")),
        ),
        (
            Format::Csv,
            "crypto",
            csv("EUR"),
            Some(fiat("csv row 2", "EUR")),
        ),
        (
            Format::Csv,
            "crypto",
            csv("SATS"),
            Some(fiat("csv row 2", "SATS")),
        ),
        (
            Format::Csv,
            "crypto",
            csv("XBT"),
            Some((
                "csv row 2 has asset 'XBT', another name for BTC".to_owned(),
                Some("Kassiber stores Bitcoin as BTC: write BTC in the row's asset column, then import the file again.".to_owned()),
            )),
        ),
        (
            Format::BtcpayJson,
            "bitcoin",
            btcpay("LTC"),
            Some(scoped("btcpay_json row 2", "LTC")),
        ),
        (Format::Csv, "crypto", csv("ETH"), None),
        (Format::Csv, "bitcoin", csv("L-BTC"), None),
        (Format::Csv, "bitcoin", csv(liquid), None),
        (Format::BtcpayJson, "bitcoin", btcpay("BTC"), None),
    ] {
        let conn = fixture();
        assets::Scope::parse(scope)
            .unwrap()
            .store(&conn, "p", "wallet")
            .unwrap();
        let result = import_kind(&conn, &content, format);
        let stored = scalar(&conn, "SELECT count(*) FROM transactions");
        match refusal {
            Some((message, hint)) => {
                let error = result.unwrap_err();
                assert_eq!(
                    (error.message, error.hint.as_deref().map(str::to_owned)),
                    (message, hint)
                );
                assert_eq!(stored, 0);
            }
            None => {
                result.unwrap();
                assert_eq!(stored, 2, "{content}");
            }
        }
    }
}

/// A crypto-scope quantity finer than 11 decimals is refused, even past the
/// 32 digits the reference's rounded `abs` keeps: the mappers hand the
/// provider's exact digits to the precision check.
#[test]
fn supplemental_csvs_refuse_quantities_finer_than_the_stored_unit() {
    let fine = "1.0000000000000000000000000000000000000001";
    for (format, content) in [
        (
            Format::Binance,
            format!("{DIVIDENDS}d1,{fine},ETH,1735787045000,Staking\n"),
        ),
        (
            Format::Binance,
            format!("{AUTO}2025-01-02 03:04:05,ETH,100.00 EUR,1.00 EUR,{fine} ETH,Spot\n"),
        ),
        (
            Format::LedgerLive,
            format!("{LEDGER}2025-01-03T01:02:03Z,ETH,IN,{fine},0,h2,Ethereum\n"),
        ),
        (
            Format::Coinfinity,
            format!("{COINFINITY}o1,buy,2025-01-02,100,{fine},ETH,,0,1,t1\n"),
        ),
    ] {
        let error = run(format, "crypto", &content).unwrap_err();
        assert!(
            error.message.contains("has more than 11 decimal places"),
            "{format:?}: {}",
            error.message
        );
    }
}
