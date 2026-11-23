//! Binance's Transaction History CSV through the exchange contract: the
//! complete ledger, spot trades included, filtered by the wallet's asset
//! scope (docs/reference/imports.md#binance-transaction-history-csv).
use super::*;

const HEADER: &str = "User_ID,UTC_Time,Account,Operation,Coin,Change,Remark\n";

/// Imports `rows` (after the header) into the 'Source' wallet in `scope`,
/// with `config` as the wallet's configuration; returns the result and the
/// stored rows, oldest first.
fn import(scope: &str, rows: &str, config: &str) -> (Result<Py, AppError>, Vec<String>) {
    import_after(scope, rows, config, |_| {})
}

/// [`import`] into a book `before` prepared.
fn import_after(
    scope: &str,
    rows: &str,
    config: &str,
    before: impl Fn(&Connection),
) -> (Result<Py, AppError>, Vec<String>) {
    let conn = fixture();
    before(&conn);
    conn.execute_batch("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'")
        .unwrap();
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id='wallet'",
        [config],
    )
    .unwrap();
    assets::Scope::parse(scope)
        .unwrap()
        .store(&conn, "p", "wallet")
        .unwrap();
    let file = std::env::temp_dir().join(format!(
        "kassiber-binance-history-{}-{:?}.csv",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&file, format!("{HEADER}{rows}")).unwrap();
    let request = || Request {
        profile_id: "p".into(),
        wallet: "Source".into(),
        file: file.to_str().unwrap().into(),
        format: Format::BinanceHistory,
        mode: None,
        companion: None,
        timezone: None,
    };
    let result = execute(&conn, request(), None);
    if result.is_ok() {
        // The same export again stores nothing new.
        let again = execute(&conn, request(), None).unwrap();
        assert_eq!(
            kassiber_envelope::get(&again, "imported"),
            Some(&Py::from(0))
        );
    }
    std::fs::remove_file(file).unwrap();
    let stored: Vec<String> = conn
        .prepare("SELECT external_id||' '||asset||' '||direction||' '||kind||' '||amount||' fee='||fee||' '||COALESCE(fiat_currency||' '||CAST(fiat_value_exact AS TEXT),'unpriced') FROM transactions WHERE wallet_id='wallet' ORDER BY occurred_at, external_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    (result, stored)
}

fn field(result: &Py, key: &str) -> String {
    kassiber_pyjson::dumps(
        kassiber_envelope::get(result, key).unwrap_or(&Py::Null),
        &Default::default(),
    )
    .unwrap()
}

/// Rows every scope books the same way. The export lists the newest first.
const BTC_ROWS: &str = "\
1,2025-01-09 10:00:00,Spot,Withdraw,BTC,-0.0015,
1,2025-01-08 10:00:00,Earn,Simple Earn Flexible Subscription,BTC,0.001,
1,2025-01-08 10:00:00,Spot,Simple Earn Flexible Subscription,BTC,-0.001,
1,2025-01-07 10:00:00,Earn,Simple Earn Flexible Interest,BTC,0.00000123,
1,2025-01-06 10:00:00,Spot,Transaction Fee,EUR,-0.04,
1,2025-01-06 10:00:00,Spot,Transaction Revenue,EUR,40,
1,2025-01-06 10:00:00,Spot,Transaction Sold,BTC,-0.001,
1,2025-01-05 10:00:00,Spot,Transaction Fee,BTC,-0.0000025,
1,2025-01-05 10:00:00,Spot,Transaction Buy,BTC,0.0015,
1,2025-01-05 10:00:00,Spot,Transaction Buy,BTC,0.001,
1,2025-01-05 10:00:00,Spot,Transaction Spend,EUR,-60,
1,2025-01-05 10:00:00,Spot,Transaction Spend,EUR,-40,
1,2025-01-04 10:00:00,Spot,Deposit,BTC,0.002,
1,2025-01-03 10:00:00,Spot,Fiat Deposit,EUR,500,
";

const BTC_STORED: [&str; 5] = [
    "binance:csv:2025-01-04 10:00:00:Spot:Deposit:BTC:0.002 BTC inbound deposit 200000000 fee=0 unpriced",
    // Two fills summed; the BTC fee is netted from what arrived.
    "binance:csv:2025-01-05 10:00:00:Spot:EUR-BTC BTC inbound buy 249750000 fee=0 EUR 100",
    // Proceeds net of the EUR fee.
    "binance:csv:2025-01-06 10:00:00:Spot:BTC-EUR BTC outbound sell 100000000 fee=0 EUR 39.96",
    "binance:csv:2025-01-07 10:00:00:Earn:Simple Earn Flexible Interest:BTC:0.00000123 BTC inbound income 123000 fee=0 unpriced",
    "binance:csv:2025-01-09 10:00:00:Spot:Withdraw:BTC:-0.0015 BTC outbound withdrawal 150000000 fee=0 unpriced",
];

#[test]
fn binance_history_books_btc_the_same_in_either_scope_and_counts_the_rest() {
    let others = "\
1,2025-01-10 10:00:00,Spot,Transaction Buy,ETH,0.5,
1,2025-01-10 10:00:00,Spot,Transaction Spend,USDT,-1000,
1,2025-01-11 10:00:00,Spot,Deposit,ETH,1,
1,2025-01-12 10:00:00,Spot,An operation Binance adds later,SOL,3,
";
    let (result, stored) = import("bitcoin", &format!("{others}{BTC_ROWS}"), "{}");
    let result = result.unwrap();
    assert_eq!(stored, BTC_STORED);
    // The unknown SOL operation never stops a Bitcoin wallet: it is left out
    // before its operation is read.
    assert_eq!(
        field(&result, "assets"),
        r#"{"scope": "bitcoin", "skipped": {"ETH": 2, "SOL": 1, "USDT": 1}, "excluded": {}, "next_step": "kassiber wallets update --wallet 'Source' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep ETH, SOL, USDT"}"#
    );

    // The crypto scope stores the same BTC rows and the other coins too;
    // there the unknown operation would be refused, so it is left out here.
    let crypto: String = others
        .lines()
        .take(3)
        .map(|line| format!("{line}\n"))
        .collect();
    let (result, stored) = import("crypto", &format!("{crypto}{BTC_ROWS}"), "{}");
    result.unwrap();
    let btc: Vec<&String> = stored.iter().filter(|row| row.contains(" BTC ")).collect();
    assert_eq!(btc, BTC_STORED.iter().collect::<Vec<_>>());
    assert!(stored.contains(&"binance:csv:2025-01-10 10:00:00:Spot:USDT-ETH:out USDT outbound swap 100000000000000 fee=0 unpriced".to_owned()), "{stored:?}");
    assert!(stored.contains(&"binance:csv:2025-01-10 10:00:00:Spot:USDT-ETH:in ETH inbound swap 50000000000 fee=0 unpriced".to_owned()), "{stored:?}");
}

#[test]
fn binance_history_swaps_and_third_asset_fees_follow_the_scope() {
    let swap = "\
1,2025-02-01 10:00:00,Spot,Transaction Fee,BNB,-0.001,
1,2025-02-01 10:00:00,Spot,Transaction Buy,ETH,0.2,
1,2025-02-01 10:00:00,Spot,Transaction Spend,BTC,-0.01,
";
    let (result, stored) = import("crypto", swap, "{}");
    assert_eq!(field(&result.unwrap(), "swap_pairs"), "1");
    assert_eq!(
        stored,
        [
            "binance:csv:2025-02-01 10:00:00:Spot:BTC-ETH:fee:BNB BNB outbound fee 0 fee=100000000 unpriced",
            "binance:csv:2025-02-01 10:00:00:Spot:BTC-ETH:in ETH inbound swap 20000000000 fee=0 unpriced",
            "binance:csv:2025-02-01 10:00:00:Spot:BTC-ETH:out BTC outbound swap 1000000000 fee=0 unpriced",
        ]
    );
    // The bitcoin scope keeps no ETH or BNB: the BTC leg is refused, never
    // booked alone, with the command that keeps both.
    let (result, stored) = import("bitcoin", swap, "{}");
    let error = result.unwrap_err();
    assert_eq!(
        error.message,
        "Binance CSV row 3 (2025-02-01 10:00:00 Spot Transaction Spend BTC) swaps BTC for ETH"
    );
    assert!(error.hint.unwrap().contains("asset_scope"));
    assert!(stored.is_empty());
    let fee_in_bnb = "\
1,2025-02-02 10:00:00,Spot,Transaction Fee,BNB,-0.001,
1,2025-02-02 10:00:00,Spot,Transaction Buy,BTC,0.01,
1,2025-02-02 10:00:00,Spot,Transaction Spend,EUR,-400,
";
    // A fee in a coin the bitcoin scope does not keep leaves the BTC trade
    // as the crypto scope books it; the fee is counted.
    let (result, bitcoin) = import("bitcoin", fee_in_bnb, "{}");
    assert_eq!(
        field(&result.unwrap(), "assets"),
        r#"{"scope": "bitcoin", "skipped": {"BNB": 1}, "excluded": {}, "next_step": "kassiber wallets update --wallet 'Source' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep BNB"}"#
    );
    assert_eq!(
        bitcoin,
        ["binance:csv:2025-02-02 10:00:00:Spot:EUR-BTC BTC inbound buy 1000000000 fee=0 EUR 400"]
    );
    let (result, crypto) = import("crypto", fee_in_bnb, "{}");
    result.unwrap();
    assert_eq!(
        crypto,
        [
            "binance:csv:2025-02-02 10:00:00:Spot:EUR-BTC BTC inbound buy 1000000000 fee=0 EUR 400",
            "binance:csv:2025-02-02 10:00:00:Spot:EUR-BTC:fee:BNB BNB outbound fee 0 fee=100000000 unpriced",
        ]
    );
}

/// The scope filters a trade by its principal legs: an ETH buy that pays
/// its fee in BTC is no BTC trade. Each scope books the BTC fee as the
/// same fee-only row, and only the crypto scope books the ETH.
#[test]
fn binance_history_a_btc_fee_of_another_coins_trade_is_its_own_row_in_both_scopes() {
    let rows = "\
1,2025-05-03 10:00:00,Spot,Transaction Fee,BTC,-0.00001,
1,2025-05-03 10:00:00,Spot,Transaction Buy,ETH,0.5,
1,2025-05-03 10:00:00,Spot,Transaction Spend,EUR,-1000,
";
    let fee = "binance:csv:2025-05-03 10:00:00:Spot:EUR-ETH:fee:BTC BTC outbound fee 0 fee=1000000 unpriced";
    let (result, bitcoin) = import("bitcoin", rows, "{}");
    result.unwrap();
    assert_eq!(bitcoin, [fee]);
    let (result, crypto) = import("crypto", rows, "{}");
    result.unwrap();
    assert_eq!(
        crypto,
        [
            "binance:csv:2025-05-03 10:00:00:Spot:EUR-ETH ETH inbound buy 50000000000 fee=0 EUR 1000",
            fee,
        ]
    );
}

/// A BTC row is the same in every scope: each fixture of this file that
/// both scopes import stores the same BTC rows.
#[test]
fn binance_history_btc_rows_are_identical_in_both_scopes() {
    for rows in [
        BTC_ROWS,
        "1,2025-02-02 10:00:00,Spot,Transaction Fee,BNB,-0.001,\n1,2025-02-02 10:00:00,Spot,Transaction Buy,BTC,0.01,\n1,2025-02-02 10:00:00,Spot,Transaction Spend,EUR,-400,\n",
        "1,2025-05-03 10:00:00,Spot,Transaction Fee,BTC,-0.00001,\n1,2025-05-03 10:00:00,Spot,Transaction Buy,ETH,0.5,\n1,2025-05-03 10:00:00,Spot,Transaction Spend,EUR,-1000,\n",
        "1,2025-05-05 10:00:00,Spot,Transaction Fee,BTC,-0.00001,\n1,2025-05-05 10:00:00,Spot,Transaction Sold,ETH,-0.5,\n1,2025-05-05 10:00:00,Spot,Transaction Revenue,EUR,1000,\n1,2025-05-06 10:00:00,Spot,Deposit,ETH,1,\n1,2025-05-06 10:00:00,Spot,Deposit,BTC,0.1,\n",
    ] {
        let btc = |scope: &str| -> Vec<String> {
            let (result, stored) = import(scope, rows, "{}");
            result.unwrap_or_else(|e| panic!("{scope}: {e:?}"));
            stored.into_iter().filter(|row| row.contains(" BTC ")).collect()
        };
        assert_eq!(btc("bitcoin"), btc("crypto"), "{rows}");
    }
}

/// A second that holds a buy and a sale of the same coin is never netted
/// into one trade: the export does not say which rows belong together.
#[test]
fn binance_history_never_nets_a_buy_and_a_sale_in_one_second() {
    let rows = "\
1,2025-05-01 10:00:00,Spot,Transaction Buy,BTC,0.03,
1,2025-05-01 10:00:00,Spot,Transaction Spend,EUR,-300,
1,2025-05-01 10:00:00,Spot,Transaction Sold,BTC,-0.01,
1,2025-05-01 10:00:00,Spot,Transaction Revenue,EUR,120,
";
    for scope in ["bitcoin", "crypto"] {
        let (result, stored) = import(scope, rows, "{}");
        let error = result.unwrap_err();
        assert!(
            error
                .message
                .ends_with("buys and sells BTC within one second; Kassiber cannot tell which rows belong together"),
            "{}",
            error.message
        );
        assert!(stored.is_empty(), "{stored:?}");
    }
}

/// A move between Binance wallets is left out only when both sides are in
/// the export: a debit without its credit refuses by name.
#[test]
fn binance_history_drops_only_internal_moves_whose_two_sides_match() {
    let both = "\
1,2025-05-02 10:00:00,Funding,Transfer Between Main and Funding Wallet,BTC,0.01,
1,2025-05-02 10:00:00,Spot,Transfer Between Main and Funding Wallet,BTC,-0.01,
";
    let (result, stored) = import("bitcoin", both, "{}");
    result.unwrap();
    assert!(stored.is_empty(), "{stored:?}");
    for rows in [
        "1,2025-05-02 10:00:00,Spot,Transfer Between Main and Funding Wallet,BTC,-0.01,\n",
        "1,2025-05-02 10:00:00,Funding,Transfer Between Main and Funding Wallet,BTC,0.004,\n1,2025-05-02 10:00:00,Spot,Transfer Between Main and Funding Wallet,BTC,-0.01,\n",
        "2,2025-05-02 10:00:00,Funding,Transfer Between Main and Funding Wallet,BTC,0.01,\n1,2025-05-02 10:00:00,Spot,Transfer Between Main and Funding Wallet,BTC,-0.01,\n",
    ] {
        let (result, stored) = import("bitcoin", rows, "{}");
        let error = result.unwrap_err();
        assert!(
            error.message.ends_with(
                "moves BTC between Binance wallets, and the export holds no matching row of the other side"
            ),
            "{}",
            error.message
        );
        assert!(stored.is_empty(), "{stored:?}");
    }
}

/// Each row's BTC is whole millisatoshi before any rows are summed.
#[test]
fn binance_history_checks_each_rows_precision_before_summing() {
    for rows in [
        "1,2025-05-04 10:00:00,Spot,Transaction Buy,BTC,0.000000000005,\n1,2025-05-04 10:00:00,Spot,Transaction Buy,BTC,0.000000000005,\n1,2025-05-04 10:00:00,Spot,Transaction Spend,EUR,-0.01,\n",
        "1,2025-05-04 10:00:00,Spot,Transaction Fee,BTC,-0.000000000005,\n1,2025-05-04 10:00:00,Spot,Transaction Fee,BTC,-0.000000000005,\n1,2025-05-04 10:00:00,Spot,Transaction Buy,BTC,0.01,\n1,2025-05-04 10:00:00,Spot,Transaction Spend,EUR,-400,\n",
    ] {
        let (result, stored) = import("bitcoin", rows, "{}");
        let error = result.unwrap_err();
        assert!(
            error
                .message
                .ends_with("has a BTC amount that is not a whole number of millisatoshi"),
            "{}",
            error.message
        );
        assert!(stored.is_empty(), "{stored:?}");
    }
}

/// A wallet that holds rows of another Binance source (an API sync, also
/// one run with an explicit backend that left the wallet's configuration
/// empty) takes no export: the same deposit would be stored twice.
#[test]
fn binance_history_refuses_a_wallet_holding_rows_of_another_binance_source() {
    let deposit = "1,2023-11-15 22:13:20,Spot,Deposit,BTC,0.02,\n";
    let synced = |conn: &Connection| {
        conn.execute_batch(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,description,counterparty,raw_json,created_at) VALUES('api','ws','p','wallet','d1','fp-api','2023-11-15T22:13:20Z','inbound','BTC',2000000000,0,'deposit','Binance deposit','Binance','{}','fixed')",
        )
        .unwrap();
    };
    let (result, stored) = import_after("bitcoin", deposit, "{}", synced);
    let error = result.unwrap_err();
    assert_eq!(
        error.message,
        "This wallet holds rows of another Binance source, which the Binance export cannot be matched to"
    );
    assert_eq!(stored.len(), 1, "{stored:?}");
}

#[test]
fn binance_history_refuses_what_it_cannot_book_by_row() {
    for (rows, message) in [
        (
            "1,2025-03-01 10:00:00,Spot,Transaction Buy,BTC,0.01,\n1,2025-03-01 10:00:00,Spot,Transaction Spend,EUR,-400,\n1,2025-03-01 10:00:00,Spot,Transaction Buy,ETH,1,\n1,2025-03-01 10:00:00,Spot,Transaction Spend,USDT,-2000,\n",
            "is one of several trades in one second (BTC, ETH, EUR, USDT)",
        ),
        (
            "1,2025-03-01 10:00:00,Cross Margin,Margin loan,BTC,0.01,\n",
            "is in Binance's Cross Margin account, which Kassiber does not book",
        ),
        (
            "1,2025-03-01 10:00:00,Spot,An operation Binance adds later,BTC,0.01,\n",
            "has an operation Kassiber does not map",
        ),
        (
            "1,2025-03-01 10:00:00,Spot,P2P Trading,BTC,0.01,\n",
            "is a P2P trade, whose payment happened outside Binance",
        ),
        (
            "1,2025-03-01 10:00:00,Spot,Deposit,BTC,0.000000000001,\n",
            "has a BTC amount that is not a whole number of millisatoshi",
        ),
        (
            "1,2025-03-01 10:00:00,Spot,Deposit,BTC,-0.01,\n",
            "moves the other way than its operation says",
        ),
        (
            "1,03/01/2025 10:00,Spot,Deposit,BTC,0.01,\n",
            "has time '03/01/2025 10:00', not YYYY-MM-DD HH:MM:SS in UTC",
        ),
    ] {
        let (result, stored) = import("crypto", rows, "{}");
        let error = result.unwrap_err();
        assert!(error.message.contains(message), "{message}: {}", error.message);
        assert!(stored.is_empty());
    }
}

/// Identical rows keep apart by their occurrence; the export has no ids.
/// A wallet that syncs the Binance API takes no export: the two sources
/// cannot be matched row by row.
#[test]
fn binance_history_numbers_identical_rows_and_keeps_out_of_api_wallets() {
    let twice = "1,2025-04-01 10:00:00,Spot,Deposit,BTC,0.01,\n1,2025-04-01 10:00:00,Spot,Deposit,BTC,0.01,\n";
    let (result, stored) = import("bitcoin", twice, "{}");
    result.unwrap();
    assert_eq!(
        stored,
        [
            "binance:csv:2025-04-01 10:00:00:Spot:Deposit:BTC:0.01 BTC inbound deposit 1000000000 fee=0 unpriced",
            "binance:csv:2025-04-01 10:00:00:Spot:Deposit:BTC:0.01#2 BTC inbound deposit 1000000000 fee=0 unpriced",
        ]
    );
    let (result, stored) = import("bitcoin", twice, r#"{"backend": "binance-main"}"#);
    let error = result.unwrap_err();
    assert_eq!(
        error.message,
        "This wallet syncs the Binance API, and the Binance export cannot be matched to its rows"
    );
    assert!(stored.is_empty());
    let error = import("bitcoin", "", "{}").0;
    assert!(error.is_ok());
}

/// The reverse order: a supplemental CSV into a wallet holding Transaction
/// History rows is refused, since the history already holds its rewards.
#[test]
fn binance_supplemental_refuses_a_wallet_holding_transaction_history_rows() {
    let conn = fixture();
    conn.execute_batch(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,description,counterparty,raw_json,created_at) VALUES('h','ws','p','wallet','binance:csv:abc','fp-h','2023-11-15T22:13:20Z','inbound','BTC',2000000,0,'income','Binance','Binance','{}','fixed')",
    )
    .unwrap();
    let content = "id,amount,asset,divTime,enInfo\nd1,0.00002,BTC,1700086400000,Staking\n";
    let file = std::env::temp_dir().join(format!(
        "kassiber-binance-supplemental-{}-{:?}.csv",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&file, content).unwrap();
    let request = Request {
        profile_id: "p".into(),
        wallet: "Source".into(),
        file: file.to_str().unwrap().into(),
        format: Format::Binance,
        mode: Some("full".into()),
        companion: None,
        timezone: None,
    };
    let error = execute(&conn, request, None).unwrap_err();
    std::fs::remove_file(file).unwrap();
    assert_eq!(
        error.message,
        "This wallet holds rows of Binance's Transaction History export, which the supplemental CSV cannot be matched to"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
}
