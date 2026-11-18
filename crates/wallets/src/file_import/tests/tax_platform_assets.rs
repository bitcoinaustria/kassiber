//! CoinTracking and Blockpit exports under a wallet's asset scope
//! (docs/plan/24-crypto-assets.md): the `bitcoin` scope counts what it
//! leaves out, the `crypto` scope imports other crypto assets, swaps pair
//! once, third-asset fees get their own row, and anything unmapped is refused.
use super::*;

const CT: &str =
    "Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Trade-Group,Comment,Date,Tx-ID\n";
const BP: &str = "Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount,Fee Asset,Fee Amount,Comments,Trx. ID\n";

/// A provider migration wallet ('Source') in an Austrian EUR book.
fn book(format: Format, scope: Option<&str>) -> Connection {
    let conn = fixture();
    conn.execute(
        "UPDATE wallets SET kind=? WHERE id='wallet'",
        [format.migration_kind()],
    )
    .unwrap();
    conn.execute_batch("UPDATE profiles SET tax_country='at', fiat_currency='EUR' WHERE id='p'")
        .unwrap();
    if let Some(scope) = scope {
        assets::Scope::parse(scope)
            .unwrap()
            .store(&conn, "p", "wallet")
            .unwrap();
    }
    conn
}

/// Each stored row of the wallet, in a readable line.
fn stored(conn: &Connection) -> Vec<String> {
    let mut statement = conn
        .prepare(
            "SELECT direction,asset,amount,fee,COALESCE(kind,''),COALESCE(fiat_value_exact,''),\
             CASE WHEN fiat_value_exact IS NULL THEN '' ELSE fiat_currency END,pricing_quality,COALESCE(pricing_pair,'') FROM transactions \
             WHERE wallet_id='wallet' ORDER BY occurred_at,direction DESC,asset",
        )
        .unwrap();
    statement
        .query_map([], |r| {
            Ok(format!(
                "{} {} {} fee={} {} {}{} {} {}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
            )
            .trim_end()
            .to_owned())
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn envelope(result: &Py, key: &str) -> String {
    kassiber_pyjson::dumps(
        kassiber_envelope::get(result, key).unwrap_or(&Py::Null),
        &Default::default(),
    )
    .unwrap()
}

fn pairs(conn: &Connection) -> usize {
    let scope = kassiber_transfers::Scope {
        workspace_id: "ws",
        profile_id: "p",
    };
    kassiber_transfers::paired_transaction_ids(conn, &scope)
        .unwrap()
        .len()
}

#[test]
fn bitcoin_scope_keeps_bitcoin_and_counts_what_it_skips() {
    for (format, body) in [
        (
            Format::CoinTracking,
            format!(
                "{CT}Trade,0.1,BTC,3000,EUR,,,Kraken,,,2025-02-01,t-btc\n\
                 Trade,2,ETH,4000,EUR,,,Kraken,,,2025-02-02,t-eth\n\
                 Trade,100,SOL,1,ETH,,,Kraken,,,2025-02-03,t-sol\n\
                 Deposit,50,USDT,,,,,Kraken,,,2025-02-04,\n\
                 Deposit,100,EUR,,,,,Kraken,,,2025-02-05,\n\
                 Trade,500,EUR,1,ETH,0.0001,BTC,Kraken,,,2025-02-06,\n"
            ),
        ),
        (
            Format::Blockpit,
            format!(
                "{BP}2025-02-01,Kraken,Trade,EUR,3000,BTC,0.1,,,,t-btc\n\
                 2025-02-02,Kraken,Trade,EUR,4000,ETH,2,,,,t-eth\n\
                 2025-02-03,Kraken,Trade,ETH,1,SOL,100,,,,t-sol\n\
                 2025-02-04,Kraken,Deposit,,,USDT,50,,,,\n\
                 2025-02-05,Kraken,Deposit,,,EUR,100,,,,\n\
                 2025-02-06,Kraken,Trade,ETH,1,EUR,500,BTC,0.0001,,\n"
            ),
        ),
    ] {
        let conn = book(format, None);
        let result = import_kind(&conn, &body, format).unwrap();
        // BTC behaves as before: the buy, and the standalone BTC fee row of
        // a trade the scope skips.
        assert_eq!(
            kassiber_envelope::get(&result, "imported"),
            Some(&Py::from(2))
        );
        assert_eq!(
            stored(&conn)[0],
            "inbound BTC 10000000000 fee=0 buy 3000EUR exact BTC-EUR",
            "{format:?}"
        );
        assert_eq!(
            envelope(&result, "assets"),
            r#"{"scope": "bitcoin", "skipped": {"ETH": 3, "SOL": 1, "USDT": 1}, "excluded": {}, "next_step": "kassiber wallets update --wallet 'Source' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep ETH, SOL, USDT"}"#,
            "{format:?}"
        );
        assert_eq!(envelope(&result, "swap_pairs"), "0");
    }
}

#[test]
fn bitcoin_scope_refusals_name_the_scope_that_imports_them() {
    let conn = book(Format::CoinTracking, None);
    for (row, code, message) in [
        (
            "Trade,0.5,ETH,0.01,BTC,,,Kraken,,,2025-02-01,s",
            "ETH",
            "CoinTracking row 1 trades Bitcoin against unsupported asset ETH",
        ),
        (
            "Trade,0.01,BTC,300,EUR,0.1,BNB,Binance,,,2025-02-01,f",
            "BNB",
            "CoinTracking row 1 has a fee in unsupported asset BNB",
        ),
    ] {
        let error = import_kind(&conn, &format!("{CT}{row}\n"), Format::CoinTracking).unwrap_err();
        assert_eq!(error.message, message);
        assert_eq!(
            error.hint.unwrap().to_string(),
            format!("This wallet imports Bitcoin only. To import {code} and the other crypto assets in this file, run kassiber wallets update --wallet 'Source' --config '{{\"asset_scope\": \"crypto\"}}', then import the file again.")
        );
    }
    // A Bitcoin unit or a fiat currency is no crypto asset: no scope helps.
    let error = import_kind(
        &conn,
        &format!("{CT}Trade,0.01,BTC,300,USD,1,EUR,Kraken,,,2025-02-01,u\n"),
        Format::CoinTracking,
    )
    .unwrap_err();
    assert!(!error.hint.unwrap().contains("asset_scope"));
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
}

#[test]
fn crypto_scope_imports_every_row_kind_for_any_crypto_asset() {
    let conn = book(Format::CoinTracking, Some("crypto"));
    let body = format!(
        "{CT}Trade,2,ETH,4000,EUR,0.01,ETH,Kraken,,,2025-02-02,t-eth\n\
         Trade,1000,EUR,0.5,ETH,5,EUR,Kraken,,,2025-02-03,t-sell\n\
         Staking,0.05,ETH,,,,,Kraken,,,2025-02-04,\n\
         Deposit,10,SOL,,,,,Kraken,,,2025-02-05,0xabc\n\
         Withdrawal,,,3,SOL,0.01,SOL,Kraken,,,2025-02-06,\n\
         Airdrop,100,ARB,,,,,Kraken,,,2025-02-07,\n\
         Mining,0.5,XMR,,,,,Pool,,,2025-02-08,\n\
         Income,5,USDT,,,,,Kraken,,,2025-02-09,\n\
         Lost,,,1,ARB,,,Kraken,,,2025-02-10,\n\
         Deposit,100,EUR,,,,,Kraken,,,2025-02-11,\n"
    );
    let result = import_kind(&conn, &body, Format::CoinTracking).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(9))
    );
    assert_eq!(
        stored(&conn),
        [
            // Fiat trades carry the exchange's execution value, net of a fiat fee.
            "inbound ETH 199000000000 fee=0 buy 4000EUR exact ETH-EUR",
            "outbound ETH 50000000000 fee=0 sell 995EUR exact ETH-EUR",
            // Everything else stays unpriced until priced in review.
            "inbound ETH 5000000000 fee=0 staking  missing",
            "inbound SOL 1000000000000 fee=0 deposit  missing",
            "outbound SOL 300000000000 fee=1000000000 withdrawal  missing",
            "inbound ARB 10000000000000 fee=0 airdrop  missing",
            "inbound XMR 50000000000 fee=0 mining  missing",
            "inbound USDT 500000000000 fee=0 income  missing",
            "outbound ARB 100000000000 fee=0 lost  missing",
        ]
    );
    assert_eq!(
        envelope(&result, "assets"),
        r#"{"scope": "crypto", "skipped": {}, "excluded": {}}"#
    );
    // Blockpit labels map the same way.
    let conn = book(Format::Blockpit, Some("crypto"));
    let body = format!(
        "{BP}2025-02-02,Kraken,Trade,EUR,4000,ETH,2,ETH,0.01,,t-eth\n\
         2025-02-04,Kraken,Staking,,,ETH,0.05,,,,\n\
         2025-02-05,Kraken,Deposit,,,SOL,10,,,,\n\
         2025-02-06,Kraken,Withdrawal,SOL,3,,,SOL,0.01,,\n\
         2025-02-07,Kraken,Airdrop,,,ARB,100,,,,\n\
         2025-02-08,Pool,Mining,,,XMR,0.5,,,,\n\
         2025-02-09,Kraken,Hard Fork,,,ETHW,2,,,,\n"
    );
    import_kind(&conn, &body, Format::Blockpit).unwrap();
    assert_eq!(
        stored(&conn),
        [
            "inbound ETH 199000000000 fee=0 buy 4000EUR exact ETH-EUR",
            "inbound ETH 5000000000 fee=0 staking  missing",
            "inbound SOL 1000000000000 fee=0 deposit  missing",
            "outbound SOL 300000000000 fee=1000000000 withdrawal  missing",
            "inbound ARB 10000000000000 fee=0 airdrop  missing",
            "inbound XMR 50000000000 fee=0 mining  missing",
            "inbound ETHW 200000000000 fee=0 hardfork  missing",
        ]
    );
}

#[test]
fn crypto_swaps_pair_once_and_reimports_add_nothing() {
    for (format, body) in [
        (
            Format::CoinTracking,
            format!(
                "{CT}Trade,0.1,BTC,3000,EUR,,,Kraken,,,2025-02-01,t-btc\n\
                 Trade,1.5,ETH,0.05,BTC,0.0001,BTC,Kraken,,,2025-03-01,\n\
                 Trade,1.5,ETH,0.05,BTC,0.0001,BTC,Kraken,,,2025-03-01,\n\
                 Trade,100,USDC,0.04,ETH,1,USDC,Kraken,,,2025-03-02,t-usdc\n"
            ),
        ),
        (
            Format::Blockpit,
            format!(
                "{BP}2025-02-01,Kraken,Trade,EUR,3000,BTC,0.1,,,,t-btc\n\
                 2025-03-01,Kraken,Trade,BTC,0.05,ETH,1.5,BTC,0.0001,,\n\
                 2025-03-01,Kraken,Trade,BTC,0.05,ETH,1.5,BTC,0.0001,,\n\
                 2025-03-02,Kraken,Trade,ETH,0.04,USDC,100,USDC,1,,t-usdc\n"
            ),
        ),
    ] {
        let conn = book(format, Some("crypto"));
        let first = import_kind(&conn, &body, format).unwrap();
        assert_eq!(
            kassiber_envelope::get(&first, "imported"),
            Some(&Py::from(7))
        );
        assert_eq!(envelope(&first, "swap_pairs"), "3", "{format:?}");
        assert_eq!(pairs(&conn), 6);
        assert_eq!(
            stored(&conn),
            [
                "inbound BTC 10000000000 fee=0 buy 3000EUR exact BTC-EUR",
                // A fee in a leg's own asset stays on that leg; two identical
                // trades stay two swaps.
                "outbound BTC 5000000000 fee=10000000 swap  missing",
                "outbound BTC 5000000000 fee=10000000 swap  missing",
                "inbound ETH 150000000000 fee=0 swap  missing",
                "inbound ETH 150000000000 fee=0 swap  missing",
                "outbound ETH 4000000000 fee=0 swap  missing",
                "inbound USDC 9900000000000 fee=0 swap  missing",
            ],
            "{format:?}"
        );
        // Each swap's legs share a reference and are told apart by direction.
        let prefix = format.migration_kind();
        assert_eq!(
            scalar(
                &conn,
                &format!(
                    "SELECT count(*) FROM transactions o JOIN transactions i \
                  ON i.external_id=substr(o.external_id,1,length(o.external_id)-4)||':in' \
                  AND i.direction='inbound' AND o.direction='outbound' \
                  WHERE o.external_id LIKE '{prefix}-swap:%:out'"
                )
            ),
            3
        );
        let again = import_kind(&conn, &body, format).unwrap();
        assert_eq!(
            kassiber_envelope::get(&again, "imported"),
            Some(&Py::from(0))
        );
        assert_eq!(envelope(&again, "swap_pairs"), "0");
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 7);
        assert_eq!(pairs(&conn), 6);
    }
}

#[test]
fn third_asset_fees_become_their_own_rows() {
    let conn = book(Format::CoinTracking, Some("crypto"));
    let body = format!(
        "{CT}Trade,0.1,BTC,3000,EUR,0.5,BNB,Binance,,,2025-02-01,t-1\n\
         Trade,2,ETH,0.1,BTC,0.2,BNB,Binance,,,2025-02-02,\n\
         Trade,100,USDC,0.04,ETH,0.00001,BTC,Binance,,,2025-02-03,\n"
    );
    let result = import_kind(&conn, &body, Format::CoinTracking).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(8))
    );
    assert_eq!(
        stored(&conn),
        [
            "outbound BNB 0 fee=50000000000 fee  missing",
            "inbound BTC 10000000000 fee=0 buy 3000EUR exact BTC-EUR",
            "outbound BNB 0 fee=20000000000 fee  missing",
            "outbound BTC 10000000000 fee=0 swap  missing",
            "inbound ETH 200000000000 fee=0 swap  missing",
            "outbound BTC 0 fee=1000000 fee  missing",
            "outbound ETH 4000000000 fee=0 swap  missing",
            "inbound USDC 10000000000000 fee=0 swap  missing",
        ]
    );
    // A fiat trade's fee row shares the trade's provider id; a swap's fee
    // row shares the swap's reference.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE external_id='t-1'"
        ),
        2
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transactions WHERE kind='fee' AND external_id LIKE 'cointracking-swap:%:fee'"),
        2
    );
    assert_eq!(envelope(&result, "swap_pairs"), "2");
    let again = import_kind(&conn, &body, Format::CoinTracking).unwrap();
    assert_eq!(
        kassiber_envelope::get(&again, "imported"),
        Some(&Py::from(0))
    );
}

#[test]
fn crypto_scope_refuses_what_it_cannot_map_and_names_the_row() {
    let conn = book(Format::CoinTracking, Some("crypto"));
    for (row, message) in [
        // Unknown types and tickers.
        (
            "Margin Trade,1,ETH,2000,EUR,,,Kraken,,,2025-02-01,",
            "CoinTracking transaction type 'Margin Trade' needs review before import",
        ),
        (
            "Deposit,1000,SATS,,,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1 has unsupported asset SATS",
        ),
        (
            "Trade,1,ETH,15000000,IDR,,,Indodax,,,2025-02-01,",
            "CoinTracking row 1 has unsupported asset IDR",
        ),
        (
            "Deposit,1,XAU,,,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1 has unsupported asset XAU",
        ),
        (
            "Deposit,1,ET H,,,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1 has unsupported asset ET H",
        ),
        // Finer than 11 decimals, on a leg and on a third-asset fee.
        (
            "Deposit,1.000000000001,ETH,,,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1: ETH amount 1.000000000001 has more than 11 decimal places",
        ),
        (
            "Trade,1,ETH,0.1,BTC,0.000000000001,BNB,Kraken,,,2025-02-01,",
            "CoinTracking row 1: BNB fee 1E-12 has more than 11 decimal places",
        ),
        // A tail past the 32 digits a rounded operation keeps is still found.
        (
            "Deposit,1.0000000000000000000000000000000000000001,ETH,,,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1: ETH amount 1.0000000000000000000000000000000000000001 has more than 11 decimal places",
        ),
        (
            "Trade,1.0000000000000000000000000000000000000001,ETH,0.1,BTC,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1: ETH amount 1.0000000000000000000000000000000000000001 has more than 11 decimal places",
        ),
        (
            "Trade,1,ETH,0.1,BTC,0.5000000000000000000000000000000000000001,BNB,Kraken,,,2025-02-01,",
            "CoinTracking row 1: BNB fee 0.5000000000000000000000000000000000000001 has more than 11 decimal places",
        ),
        // Swaps: only a trade, with two assets, both amounts, and no fiat fee.
        (
            "Deposit,1,ETH,0.1,BTC,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1 exchanges BTC for ETH under type 'Deposit'",
        ),
        (
            "Trade,1,ETH,1,ETH,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1 trades ETH for itself",
        ),
        (
            "Trade,,ETH,0.1,BTC,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1: a swap needs both amounts, but its ETH amount is missing or zero",
        ),
        (
            "Trade,1,ETH,0.1,BTC,2,EUR,Kraken,,,2025-02-01,",
            "CoinTracking row 1 has a fee in unsupported asset EUR",
        ),
        (
            "Trade,1,ETH,0.1,BTC,2,,Kraken,,,2025-02-01,",
            "CoinTracking row 1 has a fee in unsupported asset None",
        ),
        // A fiat fee in another currency than the trade's.
        (
            "Trade,1,ETH,2000,EUR,2,USD,Kraken,,,2025-02-01,",
            "CoinTracking row 1 has a fee in unsupported asset USD",
        ),
        // Bitcoin on both sides stays refused.
        (
            "Trade,1,LBTC,1,BTC,,,Kraken,,,2025-02-01,",
            "CoinTracking row 1 has Bitcoin on both sides",
        ),
    ] {
        let error = import_kind(&conn, &format!("{CT}{row}\n"), Format::CoinTracking).unwrap_err();
        assert_eq!(error.code, "validation", "{row}");
        assert_eq!(error.message, message, "{row}");
    }
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
}

#[test]
fn turning_the_scope_on_keeps_bitcoin_rows_and_adds_the_rest() {
    let conn = book(Format::Blockpit, None);
    let body = format!(
        "{BP}2025-02-01,Kraken,Trade,EUR,3000,BTC,0.1,,,,t-btc\n\
         2025-02-02,Kraken,Trade,EUR,4000,ETH,2,,,,t-eth\n\
         2025-02-03,Kraken,Deposit,,,BTC,0.2,,,,\n\
         2025-02-04,Kraken,Staking,,,ETH,0.05,,,,\n"
    );
    let first = import_kind(&conn, &body, Format::Blockpit).unwrap();
    assert_eq!(
        kassiber_envelope::get(&first, "imported"),
        Some(&Py::from(2))
    );
    let before = stored(&conn);
    assets::Scope::Crypto.store(&conn, "p", "wallet").unwrap();
    let second = import_kind(&conn, &body, Format::Blockpit).unwrap();
    assert_eq!(
        kassiber_envelope::get(&second, "imported"),
        Some(&Py::from(2))
    );
    let after = stored(&conn);
    assert!(before.iter().all(|row| after.contains(row)), "{after:?}");
    assert_eq!(after.len(), 4);
}

/// docs/plan/24-crypto-assets.md: another asset arriving with a fee in
/// itself stores what the wallet gained, so the book holds the net quantity
/// (custody counts only an inbound row's amount); the buy's value covers it.
/// A swap's inbound leg does the same. Bitcoin rows keep their fee.
#[test]
fn inbound_fees_in_the_same_asset_leave_the_net_quantity() {
    let conn = book(Format::CoinTracking, Some("crypto"));
    let body = format!(
        "{CT}Trade,100,DOT,500,EUR,0.1,DOT,Kraken,,,2025-02-01,b-1\n\
         Trade,2,ETH,0.1,BTC,0.02,ETH,Kraken,,,2025-02-02,\n\
         Deposit,0.5,BTC,,,0.0001,BTC,Kraken,,,2025-02-03,d-1\n"
    );
    import_kind(&conn, &body, Format::CoinTracking).unwrap();
    assert_eq!(
        stored(&conn),
        [
            "inbound DOT 9990000000000 fee=0 buy 500EUR exact DOT-EUR",
            "outbound BTC 10000000000 fee=0 swap  missing",
            "inbound ETH 198000000000 fee=0 swap  missing",
            "inbound BTC 50000000000 fee=10000000 deposit  missing",
        ]
    );
    // A fee that takes the whole amount is refused, naming the row.
    let conn = book(Format::CoinTracking, Some("crypto"));
    let error = import_kind(
        &conn,
        &format!("{CT}Deposit,1,ETH,,,1,ETH,Kraken,,,2025-02-01,\n"),
        Format::CoinTracking,
    )
    .unwrap_err();
    assert_eq!(
        error.message,
        "CoinTracking row 1 receives 1 ETH but charges a fee of 1 ETH"
    );
}

/// `ui.wallets.import_preview` (docs/reference/imports.md#choosing-the-assets-before-the-first-import):
/// the other crypto assets a file holds, read before its wallet exists.
#[test]
fn the_preview_names_other_crypto_assets_and_whether_bitcoin_only_can_import() {
    let preview = |content: &str, format: Format| {
        let file = std::env::temp_dir().join(format!(
            "kassiber-preview-{}-{:?}.csv",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&file, content).unwrap();
        let result = preview_assets(file.to_str().unwrap(), format, "History");
        std::fs::remove_file(file).unwrap();
        result.map(|r| {
            kassiber_pyjson::dumps(&r, &kassiber_pyjson::Options::new().sort_keys(true)).unwrap()
        })
    };
    let mixed = format!(
        "{CT}Trade,0.1,BTC,3000,EUR,,,Kraken,,,2025-02-01,t-btc\n\
         Trade,2,ETH,4000,EUR,,,Kraken,,,2025-02-02,t-eth\n\
         Deposit,3,ETH,,,,,Kraken,,,2025-02-03,d-eth\n\
         Deposit,10,SOL,,,,,Kraken,,,2025-02-04,d-sol\n"
    );
    assert_eq!(
        preview(&mixed, Format::CoinTracking).unwrap(),
        r#"{"bitcoin_only": true, "choice": true, "excluded": {}, "other_assets": {"ETH": 2, "SOL": 1}, "source_format": "cointracking_csv", "unread": {}}"#
    );
    // A Bitcoin-only history asks nothing.
    let bitcoin = format!("{CT}Trade,0.1,BTC,3000,EUR,,,Kraken,,,2025-02-01,t-btc\n");
    assert!(preview(&bitcoin, Format::CoinTracking)
        .unwrap()
        .contains(r#""choice": false"#));
    // A trade of BTC against another coin: only the crypto scope imports it.
    let swap = format!("{CT}Trade,100,SOL,0.1,BTC,,,Kraken,,,2025-02-03,t-sol\n");
    let swapped = preview(&swap, Format::CoinTracking).unwrap();
    assert!(swapped.contains(r#""bitcoin_only": false"#), "{swapped}");
    assert!(
        swapped.contains(r#""choice": true"#) && swapped.contains(r#""SOL": 1"#),
        "{swapped}"
    );
    assert!(swapped.contains(r#""bitcoin_only_error": ""#), "{swapped}");
    // Bitcoin readable, the crypto scope not (a fiat fee on a crypto trade):
    // the other assets are named with the reason, not dropped silently.
    let fiat_fee = format!("{CT}Trade,100,SOL,1,ETH,1,EUR,Kraken,,,2025-02-03,t-x\nTrade,0.1,BTC,3000,EUR,,,Kraken,,,2025-02-01,t-btc\n");
    let refused = preview(&fiat_fee, Format::CoinTracking).unwrap();
    assert!(
        refused.contains(r#""choice": false"#) && refused.contains(r#""crypto_error": ""#),
        "{refused}"
    );
    // The token binds the file's bytes.
    let file = std::env::temp_dir().join(format!("kassiber-token-{}.csv", std::process::id()));
    std::fs::write(&file, &mixed).unwrap();
    let path = file.to_str().unwrap();
    let token = file_preview_token("p", Format::CoinTracking, path).unwrap();
    check_file_preview("p", Format::CoinTracking, path, &token).unwrap();
    assert_ne!(
        file_preview_token("q", Format::CoinTracking, path).unwrap(),
        token
    );
    std::fs::write(
        &file,
        format!("{mixed}Deposit,1,DOT,,,,,Kraken,,,2025-02-05,d-dot\n"),
    )
    .unwrap();
    assert_eq!(
        check_file_preview("p", Format::CoinTracking, path, &token)
            .unwrap_err()
            .code,
        "import_preview_stale"
    );
    std::fs::remove_file(file).unwrap();
    // A generic file keeps no tally and offers nothing.
    assert!(preview("date,amount\n2025-01-01,1\n", Format::Csv)
        .unwrap()
        .contains(r#""choice": false"#));
}
