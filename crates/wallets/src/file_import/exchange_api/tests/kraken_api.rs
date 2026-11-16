//! Kraken's ledger in each asset scope (docs/plan/24-crypto-assets.md), over
//! the signed loopback fake. `bitcoin` scope is the reference mapping the
//! Python vectors pin; these tests pin the tally and the `crypto` scope.
use super::*;
use serde_json::Value as J;

/// A Kraken that answers `Ledgers` with `ledger` and `TradesHistory` with
/// `trades`, each in one page.
fn kraken(ledger: Vec<(&str, J)>, trades: Vec<(&str, J)>) -> loopback::Fake {
    let page = |rows: &[(&str, J)]| {
        let rows: serde_json::Map<String, J> = rows
            .iter()
            .map(|(id, row)| ((*id).into(), row.clone()))
            .collect();
        (rows.len(), J::Object(rows))
    };
    let (ledger, trades) = (page(&ledger), page(&trades));
    serve("kraken", move |_, _, target| {
        let (field, (count, rows)) = if target.starts_with("/0/private/TradesHistory") {
            ("trades", &trades)
        } else {
            ("ledger", &ledger)
        };
        (
            200,
            json!({"error": [], "result": {field: rows, "count": count}}).to_string(),
        )
    })
}

/// A ledger row at `time`.
fn row(kind: &str, asset: &str, amount: &str, fee: &str, refid: &str, time: i64) -> J {
    let mut row = json!({"type": kind, "asset": asset, "amount": amount, "fee": fee, "time": time, "aclass": "currency"});
    if !refid.is_empty() {
        row["refid"] = json!(refid);
    }
    row
}
fn with(mut row: J, key: &str, value: &str) -> J {
    row[key] = json!(value);
    row
}

/// The Kraken wallet a sync finds by label, in `scope`.
fn wallet(scope: &'static str) -> impl Fn(&Connection) {
    move |conn: &Connection| {
        conn.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('kw','ws','p','Kraken','kraken',?,'2025-01-01')",
            [json!({"backend": "fixture", "asset_scope": scope}).to_string()],
        )
        .unwrap();
        conn.execute_batch("UPDATE profiles SET tax_country='at' WHERE id='p'")
            .unwrap();
    }
}

fn sync(fake: &loopback::Fake, scope: &'static str) -> loopback::Run {
    loopback::run_prepared(
        &backend("kraken", &fake.url),
        "kraken",
        "",
        true,
        online(),
        &wallet(scope),
    )
}

fn dumped(value: &Py) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}

/// `(asset, direction, kind, amount, fee, fiat_value)` of one stored row.
fn stored(conn: &Connection, id: &str) -> (String, String, String, i64, i64, Option<String>) {
    conn.query_row(
        "SELECT asset,direction,kind,amount,fee,CAST(fiat_value_exact AS TEXT) FROM transactions WHERE external_id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
    )
    .unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn pairs(conn: &Connection) -> String {
    let scope = kassiber_transfers::Scope {
        workspace_id: "ws",
        profile_id: "p",
    };
    let listed = kassiber_transfers::execute(
        conn,
        &scope,
        "transfers.list",
        &dict(Vec::<(&str, Py)>::new()),
    )
    .unwrap();
    dumped(&listed)
}

/// A BTC buy, a BTC deposit and every other kind of row Kraken reports.
fn mixed_ledger() -> Vec<(&'static str, J)> {
    vec![
        ("D1", row("deposit", "XXBT", "0.02", "0", "", 1_700_000_000)),
        ("L1", row("trade", "XXBT", "0.01", "0", "T1", 1_700_000_100)),
        (
            "L1E",
            row("trade", "ZEUR", "-400", "1.2", "T1", 1_700_000_100),
        ),
        ("L1K", row("trade", "KFEE", "0", "120", "T1", 1_700_000_100)),
        ("D2", row("deposit", "XETH", "1.5", "0", "", 1_700_000_200)),
        (
            "S1",
            row("staking", "DOT.S", "0.25", "0", "", 1_700_000_300),
        ),
        ("ZE", row("deposit", "ZEUR", "1000", "0", "", 1_700_000_400)),
        (
            "E1",
            with(
                row("earn", "XBT.M", "0.0001", "0", "", 1_700_000_500),
                "subtype",
                "reward",
            ),
        ),
    ]
}
fn fiat_trade() -> Vec<(&'static str, J)> {
    vec![(
        "T1",
        json!({"pair": "XXBTZEUR", "cost": "400.00", "fee": "1.20", "price": "40000.00"}),
    )]
}

#[test]
fn kraken_bitcoin_scope_keeps_btc_and_counts_every_other_asset() {
    let fake = kraken(mixed_ledger(), fiat_trade());
    let run = sync(&fake, "bitcoin");
    let result = run.result.unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(2), "{result:?}");
    assert_eq!(
        dumped(at(&result, "assets")),
        r#"{"scope": "bitcoin", "skipped": {"BTC": 1, "DOT": 1, "ETH": 1, "KFEE": 1}, "excluded": {}, "next_step": "kassiber wallets update --wallet 'Kraken' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep BTC, DOT, ETH, KFEE"}"#
    );
    assert_eq!(at(&result, "swap_pairs"), &Py::from(0));
    // The coordinator's own count; nothing moved between Kraken balances.
    assert_eq!(get(&result, "skipped"), Some(&Py::from(0)));
    assert_eq!(
        count(
            &run.conn,
            "SELECT count(*) FROM transactions WHERE asset<>'BTC'"
        ),
        0
    );

    // Turning the scope on and syncing again keeps the BTC rows as they are
    // and adds the rest.
    let btc = |conn: &Connection| (stored(conn, "kraken:D1"), stored(conn, "kraken:L1"));
    let before = btc(&run.conn);
    assets::Scope::Crypto.store(&run.conn, "p", "kw").unwrap();
    let again = loopback::sync(
        &run.conn,
        &backend("kraken", &fake.url),
        "kraken",
        "",
        true,
        online(),
    )
    .unwrap();
    assert_eq!(at(&again, "imported"), &Py::from(4), "{again:?}");
    assert_eq!(btc(&run.conn), before);
    assert_eq!(
        dumped(at(&again, "assets")),
        r#"{"scope": "crypto", "skipped": {}, "excluded": {}}"#
    );
}

#[test]
fn kraken_crypto_scope_maps_trades_swaps_rewards_fees_and_internal_moves() {
    let mut ledger = mixed_ledger();
    ledger.extend([
        (
            "W1",
            row("withdrawal", "XETH", "-0.5", "0.001", "", 1_700_001_000),
        ),
        // An ETH sell against EUR, priced from its trade.
        ("L2", row("trade", "XETH", "-0.2", "0", "T2", 1_700_001_100)),
        ("L2E", row("trade", "ZEUR", "600", "1", "T2", 1_700_001_100)),
        // BTC for DOT: a swap, each leg with its own fee, and a KFEE fee.
        (
            "L3",
            row("trade", "XXBT", "-0.005", "0.00001", "T3", 1_700_001_200),
        ),
        (
            "L3D",
            row("trade", "DOT", "100", "0.1", "T3", 1_700_001_200),
        ),
        ("L3K", row("trade", "KFEE", "0", "25", "T3", 1_700_001_200)),
        // Buy Crypto: EUR spent, SOL received.
        (
            "R1",
            row("spend", "ZEUR", "-100", "1.5", "B1", 1_700_001_300),
        ),
        ("R2", row("receive", "SOL", "0.5", "0", "B1", 1_700_001_300)),
        // Moves between the user's own spot, staking and earn balances.
        (
            "X1",
            with(
                row("transfer", "DOT", "-50", "0", "M1", 1_700_001_400),
                "subtype",
                "spottostaking",
            ),
        ),
        (
            "X2",
            with(
                row("transfer", "DOT28.S", "50", "0", "M1", 1_700_001_400),
                "subtype",
                "stakingfromspot",
            ),
        ),
        (
            "A1",
            with(
                row("earn", "XBT.M", "0.001", "0", "", 1_700_001_500),
                "subtype",
                "allocation",
            ),
        ),
    ]);
    let mut trades = fiat_trade();
    trades.extend([
        (
            "T2",
            json!({"pair": "XETHZEUR", "cost": "600", "fee": "1", "price": "3000"}),
        ),
        ("T3", json!({"pair": "DOTXBT", "cost": "0.005", "fee": "0"})),
    ]);
    let fake = kraken(ledger, trades);
    let run = sync(&fake, "crypto");
    let result = run.result.unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(12), "{result:?}");
    assert_eq!(at(&result, "swap_pairs"), &Py::from(1));
    assert_eq!(
        dumped(at(&result, "assets")),
        r#"{"scope": "crypto", "skipped": {}, "excluded": {}}"#
    );
    assert_eq!(dumped(at(&result, "skipped")), r#"{"internal_moves": 3}"#);
    let conn = &run.conn;
    let unit = 100_000_000_000;
    let row = |id: &str| stored(conn, &format!("kraken:{id}"));
    // BTC rows are what the bitcoin scope stores.
    assert_eq!(
        row("L1"),
        (
            "BTC".into(),
            "inbound".into(),
            "buy".into(),
            unit / 100,
            0,
            Some("400.00".into())
        )
    );
    assert_eq!(
        row("D2"),
        (
            "ETH".into(),
            "inbound".into(),
            "deposit".into(),
            15 * unit / 10,
            0,
            None
        )
    );
    assert_eq!(
        row("W1"),
        (
            "ETH".into(),
            "outbound".into(),
            "withdrawal".into(),
            unit / 2,
            unit / 1000,
            None
        )
    );
    // A sell is valued net of the trade fee, as BTC sells are.
    assert_eq!(
        row("L2"),
        (
            "ETH".into(),
            "outbound".into(),
            "sell".into(),
            unit / 5,
            0,
            Some("599".into())
        )
    );
    assert_eq!(
        row("L3"),
        (
            "BTC".into(),
            "outbound".into(),
            "swap".into(),
            unit / 200,
            unit / 100_000,
            None
        )
    );
    assert_eq!(
        row("L3D"),
        (
            "DOT".into(),
            "inbound".into(),
            "swap".into(),
            100 * unit - unit / 10,
            0,
            None
        )
    );
    // Third-asset fees are their own outbound fee-only rows.
    assert_eq!(
        row("L3K"),
        (
            "KFEE".into(),
            "outbound".into(),
            "fee".into(),
            0,
            25 * unit,
            None
        )
    );
    assert_eq!(
        row("L1K"),
        (
            "KFEE".into(),
            "outbound".into(),
            "fee".into(),
            0,
            120 * unit,
            None
        )
    );
    assert_eq!(
        row("R2"),
        (
            "SOL".into(),
            "inbound".into(),
            "buy".into(),
            unit / 2,
            0,
            Some("100".into())
        )
    );
    // Staking and earn rewards are income in the base asset, unpriced.
    assert_eq!(
        row("S1"),
        (
            "DOT".into(),
            "inbound".into(),
            "income".into(),
            unit / 4,
            0,
            None
        )
    );
    assert_eq!(
        row("E1"),
        (
            "BTC".into(),
            "inbound".into(),
            "income".into(),
            unit / 10_000,
            0,
            None
        )
    );
    for skipped in [
        "kraken:X1",
        "kraken:X2",
        "kraken:A1",
        "kraken:ZE",
        "kraken:L1E",
        "kraken:R1",
    ] {
        assert_eq!(
            count(
                conn,
                &format!("SELECT count(*) FROM transactions WHERE external_id='{skipped}'")
            ),
            0,
            "{skipped}"
        );
    }
    let rate: String = conn
        .query_row(
            "SELECT CAST(fiat_rate_exact AS TEXT) FROM transactions WHERE external_id='kraken:R2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(rate.starts_with("200"), "{rate}");
    let listed = pairs(conn);
    assert!(listed.contains(r#""pair_source": "provider""#), "{listed}");
    assert!(listed.contains(r#""policy": "carrying-value""#), "{listed}");

    // A re-sync stores nothing new and authors no second pair.
    let again = loopback::sync(
        conn,
        &backend("kraken", &fake.url),
        "kraken",
        "",
        true,
        online(),
    )
    .unwrap();
    assert_eq!(at(&again, "imported"), &Py::from(0), "{again:?}");
    assert_eq!(at(&again, "swap_pairs"), &Py::from(0));
    assert_eq!(pairs(conn), listed);
}

#[test]
fn kraken_asset_codes_map_exactly_or_not_at_all() {
    for (raw, expected) in [
        ("XXBT", Some("BTC")),
        ("XBT.M", Some("BTC")),
        ("XETH", Some("ETH")),
        ("ZEUR", Some("EUR")),
        ("ZCHF", Some("CHF")),
        ("XXDG", Some("DOGE")),
        ("ZEUS", Some("ZEUS")),
        ("XTZ", Some("XTZ")),
        ("DOT.S", Some("DOT")),
        ("DOT28.S", Some("DOT")),
        ("ETH2.S", Some("ETH2")),
        ("ETH2", Some("ETH2")),
        ("USD.HOLD", Some("USD")),
        ("ADA.P", Some("ADA")),
        ("SOL.F", Some("SOL")),
        ("KSM.B", Some("KSM")),
        ("1INCH", Some("1INCH")),
        // Bonding days or part of the ticker: not guessed.
        ("ABC12.S", None),
        ("DOT.X", None),
        ("DOT28", Some("DOT28")),
        ("ET-H", None),
        ("", None),
    ] {
        assert_eq!(kraken::code(&s(raw)).as_deref(), expected, "{raw}");
    }
}

/// One `crypto`-scope sync that must be refused with `message`, storing
/// nothing.
fn refused(ledger: Vec<(&str, J)>, trades: Vec<(&str, J)>, message: &str) {
    let fake = kraken(ledger, trades);
    let run = sync(&fake, "crypto");
    let error = run.result.unwrap_err();
    assert_eq!(error.code, "validation", "{message}: {error:?}");
    assert!(
        error.message.contains(message),
        "{message}: {}",
        error.message
    );
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}

#[test]
fn kraken_crypto_scope_refuses_rows_it_cannot_map_exactly() {
    let t = 1_700_000_000;
    refused(
        vec![("M1", row("margin", "XETH", "1", "0", "", t))],
        vec![],
        "Kraken ledger row 'M1' has unsupported type 'margin'",
    );
    refused(
        vec![(
            "F1",
            with(
                row("transfer", "XETH", "-1", "0", "", t),
                "subtype",
                "spottofutures",
            ),
        )],
        vec![],
        "unsupported type 'transfer' (subtype 'spottofutures')",
    );
    refused(
        vec![(
            "F2",
            with(row("transfer", "XETH", "1", "0", "", t), "subtype", ""),
        )],
        vec![],
        "Kraken ledger row 'F2' has unsupported type 'transfer'",
    );
    refused(
        vec![("U1", row("deposit", "ABC12.S", "1", "0", "", t))],
        vec![],
        "Kraken ledger row 'U1' has asset 'ABC12.S', which Kassiber cannot map exactly",
    );
    refused(
        vec![(
            "C1",
            with(row("deposit", "XETH", "1", "0", "", t), "aclass", "futures"),
        )],
        vec![],
        "Kraken ledger row 'C1' has asset class 'futures'",
    );
    // A swap is one asset out and another in, and nothing else.
    refused(
        vec![
            ("A", row("trade", "XETH", "-1", "0", "T", t)),
            ("B", row("trade", "DOT", "50", "0", "T", t)),
            ("C", row("trade", "SOL", "2", "0", "T", t)),
        ],
        vec![],
        "Kraken trade 'T' has 3 crypto legs",
    );
    refused(
        vec![
            ("A", row("trade", "XETH", "-1", "0", "T", t)),
            ("B", row("trade", "DOT", "-50", "0", "T", t)),
        ],
        vec![],
        "Kraken swap 'T' needs one asset out and another in",
    );
    refused(
        vec![
            ("A", row("trade", "XETH", "-1", "0", "T", t)),
            ("B", row("trade", "DOT", "50", "0", "T", t)),
            ("C", row("trade", "ZEUR", "-5", "0", "T", t)),
        ],
        vec![],
        "Kraken swap 'T' also moves fiat in row 'C'",
    );
    refused(
        vec![
            ("A", row("trade", "XETH", "-1", "0", "T", t)),
            ("B", row("trade", "DOT", "50", "0", "T", t)),
            ("C", row("trade", "DOT", "0", "0.1", "T", t)),
        ],
        vec![],
        "has a separate fee row 'C' in a traded asset",
    );
    refused(
        vec![
            ("A", row("trade", "XETH", "-1", "0", "T", t)),
            ("B", row("receive", "DOT", "50", "0", "T", t)),
        ],
        vec![],
        "Kraken trade 'T' mixes trade and spend/receive rows",
    );
    refused(
        vec![("A", row("trade", "XETH", "-1", "0", "", t))],
        vec![],
        "Kraken trade row 'A' has no refid",
    );
    // A single crypto leg still needs its fiat-quoted trade, as for BTC.
    refused(
        vec![("A", row("trade", "XETH", "-1", "0", "T", t))],
        vec![("T", json!({"pair": "XETHXXBT", "cost": "0.05"}))],
        "Kraken ETH trade 'A' is not fiat-quoted",
    );
    refused(
        vec![("A", row("trade", "XETH", "-1", "0", "T", t))],
        vec![],
        "Kraken trade ledger row 'A' is missing its TradesHistory row",
    );
    refused(
        vec![
            ("A", row("spend", "ZEUR", "-10", "0", "B", t)),
            ("B", row("receive", "SOL", "1", "0", "B", t)),
            ("C", row("spend", "ZUSD", "-10", "0", "B", t)),
        ],
        vec![],
        "Kraken trade 'B' has 2 fiat legs; Buy Crypto has one",
    );
    refused(
        vec![("S", row("staking", "DOT.S", "-1", "0", "", t))],
        vec![],
        "Kraken reward row 'S' does not credit a positive amount",
    );
    refused(
        vec![(
            "X",
            with(
                row("transfer", "DOT", "-1", "0.01", "", t),
                "subtype",
                "spottostaking",
            ),
        )],
        vec![],
        "Kraken transfer row 'X' charges a fee on a move between Kraken balances",
    );
    // Finer than the 11 decimals the book stores, named by row.
    refused(
        vec![("D", row("deposit", "XETH", "0.000000000001", "0", "", t))],
        vec![],
        "Kraken ledger row 'D': ETH amount 1E-12 has more than 11 decimal places",
    );
    refused(
        vec![
            ("A", row("trade", "XETH", "-1", "0", "T", t)),
            ("B", row("trade", "DOT", "50", "0", "T", t)),
            ("C", row("trade", "KFEE", "0", "0.000000000001", "T", t)),
        ],
        vec![],
        "Kraken ledger row 'C': KFEE fee",
    );
    // A tokenized asset paid for with crypto cannot drop its crypto side.
    refused(
        vec![
            ("A", row("trade", "USDC", "-200", "0", "T", t)),
            (
                "B",
                with(
                    row("trade", "AAPLX", "1", "0", "T", t),
                    "aclass",
                    "tokenized_asset",
                ),
            ),
        ],
        vec![],
        "Kraken trade 'T' exchanges crypto for a tokenized asset",
    );
}

#[test]
fn kraken_tokenized_assets_are_excluded_by_their_own_asset_class() {
    let t = 1_700_000_000;
    let tokenized = |row: J| with(row, "aclass", "tokenized_asset");
    let ledger = vec![
        ("A", row("trade", "ZUSD", "-200", "0", "T", t)),
        ("B", tokenized(row("trade", "AAPLX", "1", "0", "T", t))),
        ("C", tokenized(row("deposit", "TSLAX", "2", "0", "", t))),
        ("D", row("deposit", "XETH", "1", "0", "", t)),
    ];
    for (scope, expected) in [
        (
            "crypto",
            r#"{"scope": "crypto", "skipped": {}, "excluded": {"AAPLX": 1, "TSLAX": 1}}"#,
        ),
        (
            "bitcoin",
            r#"{"scope": "bitcoin", "skipped": {"ETH": 1}, "excluded": {"AAPLX": 1, "TSLAX": 1}, "next_step": "kassiber wallets update --wallet 'Kraken' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep ETH"}"#,
        ),
    ] {
        let fake = kraken(ledger.clone(), vec![]);
        let run = sync(&fake, scope);
        let result = run.result.unwrap();
        assert_eq!(dumped(at(&result, "assets")), expected, "{scope}");
        assert_eq!(
            count(
                &run.conn,
                "SELECT count(*) FROM transactions WHERE asset IN ('AAPLX','TSLAX')"
            ),
            0
        );
    }
}

/// A BTC trade against another crypto asset: the reference refuses it and
/// sent the user to the generic ledger, which refuses the other leg too.
/// A sync names the command that imports both legs instead (bitcoin scope),
/// or the generic CSV/JSON import (crypto scope, other leg missing).
#[test]
fn kraken_crypto_quoted_trade_hint_names_a_way_to_import_it() {
    let t = 1_735_689_600;
    for (scope, asset, pair, hint) in [
        (
            "bitcoin",
            "XXBT",
            "XETHXXBT",
            r#"Kassiber does not book one leg of a swap as a sale or purchase. To import both legs, run kassiber wallets update --wallet 'Kraken' --config '{"asset_scope": "crypto"}' and sync again."#,
        ),
        (
            "crypto",
            "XETH",
            "XETHXXBT",
            "Kassiber did not find this trade's other leg; record the trade yourself with the generic CSV/JSON import, a swap as two rows sharing a swap_ref.",
        ),
    ] {
        let fake = kraken(
            vec![("A", row("trade", asset, "-1", "0", "T", t))],
            vec![("T", json!({"pair": pair, "cost": "0.05"}))],
        );
        let run = sync(&fake, scope);
        let error = run.result.unwrap_err();
        assert!(error.message.ends_with("trade 'A' is not fiat-quoted"), "{error:?}");
        assert_eq!(error.hint.as_deref(), Some(hint));
    }
}
