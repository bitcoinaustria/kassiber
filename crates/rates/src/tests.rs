use super::*;
use kassiber_net::{Gate, Ledger, Offline};
fn book() -> (tempfile::TempDir, kassiber_store::Book) {
    let root = tempfile::tempdir().unwrap();
    let book =
        kassiber_store::Book::open_migrating(&root.path().join("data"), Default::default(), false)
            .unwrap();
    (root, book)
}
fn args(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    dict(entries)
}
fn rate(conn: &Connection, ts: &str, value: &str, source: &str, fetched: &str) {
    upsert(
        conn,
        &Sample {
            pair: "BTC-EUR",
            timestamp: ts,
            rate: value,
            source,
            fetched_at: fetched,
            ..Default::default()
        },
    )
    .unwrap();
}
const AT: &str = "2024-05-01T00:00:00Z";
const NEXT: &str = "2024-05-02T00:00:00Z";
#[test]
fn exact_manual_upsert_invalidates_only_matching_fiat() {
    let (_root, b) = book();
    let c = b.connection();
    c.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','now'); INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('eur','w','EUR','EUR','now'),('usd','w','USD','USD','now');").unwrap();
    let a = args([
        ("pair", s("btc/eur")),
        ("timestamp", s("20240501T020000+02:00")),
        ("rate", s("65000.12345678901234567890123456789")),
    ]);
    let r = set_manual(c, &a).unwrap();
    assert_eq!(
        text(&r, "rate_exact"),
        Some("65000.12345678901234567890123456789")
    );
    assert_eq!(text(&r, "timestamp"), Some(AT));
    let versions = query(
        c,
        "SELECT id,journal_input_version FROM profiles ORDER BY id",
        [],
    )
    .unwrap();
    assert_eq!(
        get(&versions[0], "journal_input_version"),
        Some(&Value::from(1))
    );
    assert_eq!(
        get(&versions[1], "journal_input_version"),
        Some(&Value::from(0))
    );
    assert_eq!(
        text(&latest(c, "BTC-EUR").unwrap(), "source"),
        Some("manual")
    );
    set_manual(c, &a).unwrap();
    assert_eq!(query(c, "SELECT * FROM rates_cache", []).unwrap().len(), 1);
}
#[test]
fn latest_manual_then_fetched_then_source_ties_match_base() {
    let (_r, b) = book();
    let c = b.connection();
    rate(c, AT, "100", "z", AT);
    rate(c, AT, "200", "a", AT);
    assert_eq!(text(&latest(c, "btc/eur").unwrap(), "source"), Some("a"));
    rate(c, AT, "300", "z", NEXT);
    assert_eq!(text(&latest(c, "BTC-EUR").unwrap(), "source"), Some("z"));
    rate(c, AT, "400", "manual", AT);
    assert_eq!(
        text(&latest(c, "BTC-EUR").unwrap(), "source"),
        Some("manual")
    );
    rate(c, NEXT, "500", "a", AT);
    assert_eq!(
        text(&latest(c, "BTC-EUR").unwrap(), "rate_exact"),
        Some("500")
    );
}
#[test]
fn lookup_excludes_future_and_preserves_insertion_tie() {
    let (_r, b) = book();
    let c = b.connection();
    rate(c, AT, "100", "z", AT);
    rate(c, AT, "200", "a", AT);
    rate(c, NEXT, "900", "manual", NEXT);
    assert!(cached_at_or_before(c, "BTC-EUR", "2024-04-30")
        .unwrap()
        .is_none());
    let r = cached_at_or_before(c, "BTC-EUR", AT).unwrap().unwrap();
    // Python's base query has no source tie-break. SQLite's index scan is
    // retained verbatim by sharing the same schema and query order.
    // Independently pinned against Python on this base: z was inserted first.
    assert_eq!(text(&r, "source"), Some("z"));
    assert_ne!(text(&r, "timestamp"), Some(NEXT));
    rate(c, AT, "300", "manual", AT);
    assert_eq!(
        text(
            &cached_at_or_before(c, "BTC-EUR", AT).unwrap().unwrap(),
            "source"
        ),
        Some("manual")
    );
}
#[test]
fn range_inclusive_order_and_limit() {
    let (_r, b) = book();
    let c = b.connection();
    rate(c, AT, "100", "manual", AT);
    rate(c, NEXT, "200", "manual", NEXT);
    let a = args([
        ("pair", s("BTC-EUR")),
        ("start", s(AT)),
        ("end", s(NEXT)),
        ("order", s("desc")),
        ("limit", Value::from(1)),
    ]);
    let Value::List(rows) = range(c, &a).unwrap() else {
        panic!()
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(text(&rows[0], "timestamp"), Some(NEXT));
    assert!(range(
        c,
        &args([("pair", s("BTC-EUR")), ("limit", Value::from(0))])
    )
    .is_err());
}
#[test]
fn pairs_include_supported_empty_and_unknown_cached() {
    let (_r, b) = book();
    let c = b.connection();
    upsert(
        c,
        &Sample {
            pair: "ETH-GBP",
            timestamp: AT,
            rate: "12",
            source: "manual",
            fetched_at: AT,
            ..Default::default()
        },
    )
    .unwrap();
    let Value::List(rows) = pairs(c).unwrap() else {
        panic!()
    };
    assert_eq!(rows.len(), 3);
    assert_eq!(get(&rows[0], "cached"), Some(&Value::Bool(false)));
    assert_eq!(get(&rows[2], "supported"), Some(&Value::Bool(false)));
}
#[test]
fn normalized_provider_fallback_and_asset_aliases() {
    let (_r, b) = book();
    let c = b.connection();
    assert_eq!(market_provider(c).unwrap(), "coinbase-exchange");
    set_market_provider(c, " MEMPOOL ").unwrap();
    assert_eq!(market_provider(c).unwrap(), "mempool");
    assert!(set_market_provider(c, "kraken-csv").is_err());
    c.execute(
        "UPDATE settings SET value='junk' WHERE key='market_rate_provider'",
        [],
    )
    .unwrap();
    assert_eq!(market_provider(c).unwrap(), "coinbase-exchange");
    assert_eq!(transaction_rate_pair("lbtc", "eur"), Some("BTC-EUR".into()));
    assert_eq!(
        transaction_rate_pair(" l-btc ", "eur"),
        Some("BTC-EUR".into())
    );
    assert!(transaction_rate_pair("BTC", "GBP").is_none());
    assert_eq!(
        require_supported_pair("BTC-GBP").unwrap_err().code,
        "validation"
    );
}
#[test]
fn iso_microseconds_offsets_and_python_shapes() {
    for (raw, want) in [
        ("2024-05-01", AT),
        ("20240501T000000", AT),
        ("2024-W18-3T02:00:00+02:00", AT),
        (
            "2024-05-01T00:00:00.1234567Z",
            "2024-05-01T00:00:00.123456Z",
        ),
        ("2024-05-01t00:00", AT),
    ] {
        assert_eq!(timestamp(raw, "test").unwrap(), want)
    }
    for raw in ["2024-13-01", "2024-05-01T24:00:00Z", "junk"] {
        assert!(timestamp(raw, "test").is_err())
    }
}
#[test]
fn csv_selection_chooses_finest_and_first_duplicate() {
    let (root, b) = book();
    let c = b.connection();
    let dir = root.path().join("csv");
    std::fs::create_dir(&dir).unwrap();
    for name in [
        "XBTEUR_1.csv",
        "XXBTZEUR_1.csv",
        "XBTEUR_60.csv",
        "XBTEUR_1440.csv",
        "XBTUSD_60.csv",
        "XBTUSDT_1.csv",
        "ETHUSD_1.csv",
        "README.txt",
    ] {
        std::fs::write(dir.join(name), "1714521600,100,110,90,105,1,2\n").unwrap()
    }
    let Value::List(rows) = csv::sync(c, Some("BTC-EUR"), dir.to_str().unwrap()).unwrap() else {
        panic!()
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(text(&rows[0], "granularity"), Some("minute"));
    assert_eq!(get(&rows[0], "skipped_files"), Some(&Value::from(6)));
    assert_eq!(
        text(&rows[0], "last_timestamp"),
        Some("2024-05-01T00:01:00Z")
    );
    let rows=query(c,"SELECT rate_exact,open_rate_exact,high_rate_exact,low_rate_exact,close_rate_exact,volume_exact,trades FROM rates_cache",[]).unwrap();
    assert_eq!(text(&rows[0], "open_rate_exact"), Some("100"));
    assert_eq!(get(&rows[0], "trades"), Some(&Value::from(2)));
}
#[test]
fn csv_rejects_negative_ohlc_volume_trades_and_bad_rows() {
    let (root, b) = book();
    let file = root.path().join("XBTEUR_1.csv");
    let mut contents = "timestamp,open,high,low,close,volume,trades\n".to_owned();
    for column in 1..7 {
        let mut row = ["1714521600", "100", "110", "90", "105", "1", "2"];
        row[column] = "-1";
        contents += &(row.join(",") + "\n");
    }
    contents += "1714521600,100,110,90,105,1,2\n";
    std::fs::write(&file, contents).unwrap();
    let Value::List(rows) = csv::sync(b.connection(), None, file.to_str().unwrap()).unwrap() else {
        panic!()
    };
    assert_eq!(get(&rows[0], "samples"), Some(&Value::from(1)));
    assert_eq!(get(&rows[0], "skipped_rows"), Some(&Value::from(7)));
}
#[test]
fn csv_hourly_daily_close_and_idempotence() {
    let (root, b) = book();
    for (interval, at) in [(60, "2024-05-01T01:00:00Z"), (1440, "2024-05-02T00:00:00Z")] {
        let file = root.path().join(format!("XBTEUR_{interval}.csv"));
        std::fs::write(
            &file,
            "1714521600,100,110,90,105.00000000000000000001,1,2\n",
        )
        .unwrap();
        for _ in 0..2 {
            csv::sync(b.connection(), None, file.to_str().unwrap()).unwrap();
        }
        assert_eq!(
            query(
                b.connection(),
                "SELECT * FROM rates_cache WHERE timestamp=?",
                [at]
            )
            .unwrap()
            .len(),
            1
        );
    }
}
#[test]
fn csv_missing_path_errors_and_manual_validation() {
    let (_r, b) = book();
    assert_eq!(
        csv::sync(b.connection(), None, "/nonexistent-kassiber-rate-test")
            .unwrap_err()
            .code,
        "not_found"
    );
    for raw in ["0", "-1", "junk"] {
        assert!(set_manual(
            b.connection(),
            &args([
                ("pair", s("BTC-EUR")),
                ("timestamp", s(AT)),
                ("rate", s(raw))
            ])
        )
        .is_err());
    }
}
#[test]
fn rebuilt_rows_rollback_on_missing_consent() {
    let (_r, b) = book();
    let c = b.connection();
    rate(c, AT, "100", "coingecko", AT);
    let providers = Providers::new(Gate::new(|| Offline::Online, Ledger::default()), "test");
    let a = args([("source", s("coingecko")), ("pair", s("BTC-EUR"))]);
    assert_eq!(
        command(c, "rebuild", &a, &providers, None)
            .unwrap_err()
            .code,
        "interaction_required"
    );
    assert_eq!(
        text(&latest(c, "BTC-EUR").unwrap(), "rate_exact"),
        Some("100")
    );
}
#[test]
fn desktop_reads_are_local_and_live_disabled_without_feature() {
    let (_r, b) = book();
    let c = b.connection();
    let ledger = Ledger::default();
    let p = Providers::new(Gate::new(|| Offline::Online, ledger.clone()), "test");
    let empty = dict::<&str>([]);
    desktop_command(c, "ui.rates.summary", &empty, &p).unwrap();
    desktop_command(c, "ui.rates.coverage", &empty, &p).unwrap();
    c.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','now');INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','P','EUR','now');INSERT INTO settings(key,value) VALUES('context_profile','p'),('context_workspace','w');").unwrap();
    assert_eq!(
        desktop_command(c, "ui.rates.latest", &empty, &p)
            .unwrap_err()
            .code,
        "live_market_rates_disabled"
    );
    assert_eq!(
        desktop_command(c, "ui.rates.rebuild", &empty, &p)
            .unwrap_err()
            .code,
        "not_implemented"
    );
    assert!(ledger.snapshot(0, 100).records.is_empty());
}

fn seed_need(c: &Connection) {
    c.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','now');INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','P','EUR','now');INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet','w','p','Cold','address','{}','now');INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,confirmed_at,direction,asset,amount,fee,fiat_currency,kind,created_at) VALUES('tx','w','p','wallet','ext','fp','2024-05-01T12:34:56Z','2024-05-01T12:34:56Z','inbound','BTC',100000000000,0,'EUR','deposit','now');INSERT INTO settings(key,value) VALUES('context_profile','p'),('context_workspace','w');").unwrap();
}
fn fake_providers(body: &str) -> (kassiber_net::test_support::FakeProvider, Providers) {
    let server = kassiber_net::test_support::FakeProvider::new(body).unwrap();
    let mut p = Providers::new(Gate::new(|| Offline::Online, Ledger::default()), "test");
    p.test_base = Some(server.base().into());
    (server, p)
}
fn seed_mempool(c: &Connection) {
    c.execute_batch("INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at) VALUES('own','mempool','bitcoin','main','http://127.0.0.1:1/api','now','now');").unwrap();
}
#[test]
fn coinbase_latest_no_traffic_before_exact_consent() {
    let (_r, b) = book();
    let c = b.connection();
    let (server, p) = fake_providers("[[1714521600,90,110,100,105.123456789,1]]");
    assert_eq!(
        p.latest(
            c,
            Some("BTC-EUR"),
            Some("coinbase-exchange"),
            None,
            FetchAction::Latest
        )
        .unwrap_err()
        .code,
        "interaction_required"
    );
    assert!(server.requests().is_empty());
    let wrong =
        FetchConsent::explicit(c, FetchAction::Sync, "coinbase-exchange", Some("BTC-EUR")).unwrap();
    assert!(p
        .latest(
            c,
            Some("BTC-EUR"),
            Some("coinbase-exchange"),
            Some(&wrong),
            FetchAction::Latest
        )
        .is_err());
    assert!(server.requests().is_empty());
    let consent =
        FetchConsent::explicit(c, FetchAction::Latest, "coinbase-exchange", Some("BTC-EUR"))
            .unwrap();
    p.latest(
        c,
        Some("BTC-EUR"),
        Some("coinbase-exchange"),
        Some(&consent),
        FetchAction::Latest,
    )
    .unwrap();
    assert_eq!(server.requests().len(), 1);
    assert_eq!(
        text(&latest(c, "BTC-EUR").unwrap(), "rate_exact"),
        Some("105.123456789")
    );
}
#[test]
fn coingecko_sync_and_latest_no_traffic_before_consent() {
    let (_r, b) = book();
    let c = b.connection();
    let (server, p) =
        fake_providers("{\"prices\":[[1714521600000,105.125],[1714521660000,106.25]]}");
    let a = args([
        ("source", s("coingecko")),
        ("pair", s("BTC-EUR")),
        ("days", Value::from(91)),
    ]);
    for action in [FetchAction::Sync, FetchAction::Rebuild] {
        assert_eq!(
            p.sync(c, &a, None, action).unwrap_err().code,
            "interaction_required"
        )
    }
    assert!(p
        .latest(
            c,
            Some("BTC-EUR"),
            Some("coingecko"),
            None,
            FetchAction::Latest
        )
        .is_err());
    assert!(server.requests().is_empty());
    let consent =
        FetchConsent::explicit(c, FetchAction::Sync, "coingecko", Some("BTC-EUR")).unwrap();
    let Value::List(rows) = p.sync(c, &a, Some(&consent), FetchAction::Sync).unwrap() else {
        panic!()
    };
    assert_eq!(text(&rows[0], "granularity"), Some("daily"));
    assert_eq!(get(&rows[0], "samples"), Some(&Value::from(2)));
    assert_eq!(server.requests().len(), 1);
    let consent =
        FetchConsent::explicit(c, FetchAction::Latest, "coingecko", Some("BTC-EUR")).unwrap();
    p.latest(
        c,
        Some("BTC-EUR"),
        Some("coingecko"),
        Some(&consent),
        FetchAction::Latest,
    )
    .unwrap();
    assert_eq!(server.requests().len(), 2);
}
#[test]
fn coinbase_transaction_need_no_traffic_before_consent_and_incremental_cache() {
    let (_r, b) = book();
    let c = b.connection();
    seed_need(c);
    let (server, p) = fake_providers("[[1714566780,90,110,100,105,1]]");
    let a = args([
        ("source", s("coinbase-exchange")),
        ("pair", s("BTC-EUR")),
        ("days", Value::from(1)),
    ]);
    assert_eq!(
        p.sync(c, &a, None, FetchAction::Sync).unwrap_err().code,
        "interaction_required"
    );
    assert!(server.requests().is_empty());
    let consent =
        FetchConsent::explicit(c, FetchAction::Sync, "coinbase-exchange", Some("BTC-EUR")).unwrap();
    let Value::List(first) = p.sync(c, &a, Some(&consent), FetchAction::Sync).unwrap() else {
        panic!()
    };
    let Value::List(second) = p.sync(c, &a, Some(&consent), FetchAction::Sync).unwrap() else {
        panic!()
    };
    assert_eq!(server.requests().len(), 1);
    assert_eq!(get(&first[0], "missing_minutes"), Some(&Value::from(1)));
    assert_eq!(get(&second[0], "cached_minutes"), Some(&Value::from(1)));
    assert_eq!(get(&second[0], "missing_minutes"), Some(&Value::from(0)));
}
#[test]
fn coinbase_empty_window_is_checked_without_refetch() {
    let (_r, b) = book();
    let c = b.connection();
    seed_need(c);
    let (server, p) = fake_providers("[]");
    let a = args([("source", s("coinbase-exchange")), ("pair", s("BTC-EUR"))]);
    let consent =
        FetchConsent::explicit(c, FetchAction::Sync, "coinbase-exchange", Some("BTC-EUR")).unwrap();
    p.sync(c, &a, Some(&consent), FetchAction::Sync).unwrap();
    let Value::List(second) = p.sync(c, &a, None, FetchAction::Sync).unwrap() else {
        panic!()
    };
    assert_eq!(server.requests().len(), 1);
    assert_eq!(
        get(&second[0], "already_checked_minutes"),
        Some(&Value::from(1))
    );
}
#[test]
fn coinbase_idle_continuous_windows_require_consent() {
    let (_r, b) = book();
    let c = b.connection();
    let (server, p) = fake_providers("[]");
    let a = args([
        ("source", s("coinbase-exchange")),
        ("pair", s("BTC-EUR")),
        ("days", Value::from(1)),
    ]);
    assert!(p.sync(b.connection(), &a, None, FetchAction::Sync).is_err());
    assert!(server.requests().is_empty());
    let consent =
        FetchConsent::explicit(c, FetchAction::Sync, "coinbase-exchange", Some("BTC-EUR")).unwrap();
    let Value::List(rows) = p
        .sync(b.connection(), &a, Some(&consent), FetchAction::Sync)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(text(&rows[0], "mode"), Some("continuous_days"));
    assert_eq!(server.requests().len(), 5);
}
#[test]
fn mempool_latest_idle_and_historical_no_traffic_before_consent() {
    let (_r, b) = book();
    let c = b.connection();
    seed_mempool(c);
    let (server, p) = fake_providers("{\"time\":1714521600,\"EUR\":\"105.1234567890123456789\"}");
    let a = args([("source", s("mempool")), ("pair", s("BTC-EUR"))]);
    assert!(p
        .latest(
            c,
            Some("BTC-EUR"),
            Some("mempool"),
            None,
            FetchAction::Latest
        )
        .is_err());
    assert!(p.sync(c, &a, None, FetchAction::Sync).is_err());
    assert!(server.requests().is_empty());
    let consent =
        FetchConsent::explicit(c, FetchAction::Latest, "mempool", Some("BTC-EUR")).unwrap();
    p.latest(
        c,
        Some("BTC-EUR"),
        Some("mempool"),
        Some(&consent),
        FetchAction::Latest,
    )
    .unwrap();
    assert_eq!(server.requests().len(), 1);
    seed_need(c);
    assert!(p.sync(c, &a, None, FetchAction::Sync).is_err());
    assert_eq!(server.requests().len(), 1);
    let consent = FetchConsent::explicit(c, FetchAction::Sync, "mempool", Some("BTC-EUR")).unwrap();
    p.sync(c, &a, Some(&consent), FetchAction::Sync).unwrap();
    let r = latest(c, "BTC-EUR").unwrap();
    assert_eq!(text(&r, "rate_exact"), Some("105.1234567890123456789"));
    assert_eq!(text(&r, "timestamp"), Some("2024-05-01T12:34:00Z"));
    assert_eq!(text(&r, "granularity"), Some("daily"));
    assert!(server.requests()[1].contains("historical-price?currency=EUR&timestamp="));
}
#[test]
fn every_live_path_offline_blocks_before_traffic() {
    let (_r, b) = book();
    let c = b.connection();
    seed_mempool(c);
    let (server, mut p) = fake_providers("{}");
    p.gate_for_test(Offline::Offline);
    for source in LIVE_SOURCES {
        let consent =
            FetchConsent::explicit(c, FetchAction::Latest, source, Some("BTC-EUR")).unwrap();
        assert_eq!(
            p.latest(
                c,
                Some("BTC-EUR"),
                Some(source),
                Some(&consent),
                FetchAction::Latest
            )
            .unwrap_err()
            .code,
            "network_egress_disabled"
        );
    }
    seed_need(c);
    for source in LIVE_SOURCES {
        let a = args([("source", s(*source)), ("pair", s("BTC-EUR"))]);
        let consent =
            FetchConsent::explicit(c, FetchAction::Sync, source, Some("BTC-EUR")).unwrap();
        assert_eq!(
            p.sync(c, &a, Some(&consent), FetchAction::Sync)
                .unwrap_err()
                .code,
            "network_egress_disabled"
        );
    }
    assert!(server.requests().is_empty());
}
#[test]
fn every_provider_rebuild_requires_its_own_action_grant() {
    let (_r, b) = book();
    let c = b.connection();
    seed_need(c);
    seed_mempool(c);
    let (server, p) = fake_providers("{}");
    for source in LIVE_SOURCES {
        let args = args([("source", s(*source)), ("pair", s("BTC-EUR"))]);
        assert_eq!(
            command(c, "rebuild", &args, &p, None).unwrap_err().code,
            "interaction_required"
        );
        let wrong = FetchConsent::explicit(c, FetchAction::Sync, source, Some("BTC-EUR")).unwrap();
        assert_eq!(
            command(c, "rebuild", &args, &p, Some(&wrong))
                .unwrap_err()
                .code,
            "interaction_required"
        );
    }
    assert!(server.requests().is_empty());
}
#[test]
fn proxy_credentials_follow_urllib_unquote() {
    assert_eq!(
        providers::unquote("alice%40host:p%2B%25%ff%xy+"),
        "alice@host:p+%\u{fffd}%xy+"
    );
}
#[test]
fn action_grants_cannot_relabel_fetches_or_warm_background_cache() {
    let (_r, b) = book();
    let c = b.connection();
    seed_need(c);
    let (server, p) = fake_providers("{}");
    let args = args([
        ("source", s("coinbase-exchange")),
        ("pair", s("BTC-EUR")),
        ("days", Value::from(1)),
    ]);
    let latest =
        FetchConsent::explicit(c, FetchAction::Latest, "coinbase-exchange", Some("BTC-EUR"))
            .unwrap();
    assert_eq!(
        p.sync(c, &args, Some(&latest), FetchAction::Latest)
            .unwrap_err()
            .code,
        "interaction_required"
    );
    let sync =
        FetchConsent::explicit(c, FetchAction::Sync, "coinbase-exchange", Some("BTC-EUR")).unwrap();
    assert_eq!(
        p.latest(
            c,
            Some("BTC-EUR"),
            Some("coinbase-exchange"),
            Some(&sync),
            FetchAction::Sync
        )
        .unwrap_err()
        .code,
        "interaction_required"
    );
    c.execute("INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p','{\"source_classes\":{\"market_rates\":true}}')", []).unwrap();
    c.execute("UPDATE transactions SET fiat_rate=1", [])
        .unwrap();
    let enabled = FetchConsent::enabled_refresh(c, "p", "coinbase-exchange", "BTC-EUR").unwrap();
    let Value::List(rows) = p
        .sync(c, &args, Some(&enabled), FetchAction::EnabledRefresh)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(text(&rows[0], "mode"), Some("idle_no_missing_minutes"));
    let mut warm = args;
    if let Value::Dict(fields) = &mut warm {
        fields.push(("warm_cache_when_idle".into(), Value::Bool(true)));
    }
    assert_eq!(
        p.sync(c, &warm, Some(&enabled), FetchAction::EnabledRefresh)
            .unwrap_err()
            .code,
        "interaction_required"
    );
    assert!(server.requests().is_empty());
}
#[test]
fn windows_respect_midnight_max_candles_and_future() {
    let now = time::parse("2024-05-01T12:00:00Z").unwrap();
    let minutes = [
        "2024-05-01T00:00:00Z",
        "2024-05-01T00:01:00Z",
        "2024-05-01T05:00:00Z",
        "2024-05-01T12:01:00Z",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let w = providers::windows(&minutes, now);
    assert_eq!(w.len(), 2);
    assert_eq!(
        time::format(w[0].0).as_deref(),
        Some("2024-04-30T19:00:00Z")
    );
    assert_eq!(
        time::format(w[1].1).as_deref(),
        Some("2024-05-01T05:00:00Z")
    );
}

#[test]
fn background_grant_rechecks_feature_and_book_identity() {
    let (_r, b) = book();
    let c = b.connection();
    seed_need(c);
    let (server, p) = fake_providers("{\"prices\":[[1714521600000,100]]}");
    assert!(FetchConsent::enabled_refresh(c, "p", "coingecko", "BTC-EUR").is_err());
    c.execute("INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p','{\"source_classes\":{\"market_rates\":true}}')",[]).unwrap();
    let consent = FetchConsent::enabled_refresh(c, "p", "coingecko", "BTC-EUR").unwrap();
    p.latest(
        c,
        Some("BTC-EUR"),
        Some("coingecko"),
        Some(&consent),
        FetchAction::EnabledRefresh,
    )
    .unwrap();
    assert_eq!(server.requests().len(), 1);
    c.execute(
        "UPDATE settings SET value='{}' WHERE key='freshness.policy.profile.p'",
        [],
    )
    .unwrap();
    assert_eq!(
        p.latest(
            c,
            Some("BTC-EUR"),
            Some("coingecko"),
            Some(&consent),
            FetchAction::EnabledRefresh
        )
        .unwrap_err()
        .code,
        "live_market_rates_disabled"
    );
    assert_eq!(server.requests().len(), 1);
    let (_other, other) = book();
    let consent =
        FetchConsent::explicit(c, FetchAction::Latest, "coingecko", Some("BTC-EUR")).unwrap();
    assert_eq!(
        p.latest(
            other.connection(),
            Some("BTC-EUR"),
            Some("coingecko"),
            Some(&consent),
            FetchAction::Latest
        )
        .unwrap_err()
        .code,
        "interaction_required"
    );
    assert_eq!(server.requests().len(), 1);
}
#[test]
fn bundled_daily_seed_is_embedded_and_idempotent() {
    let (_r, b) = book();
    let c = b.connection();
    let Value::List(first) = ensure_bundled_seed(c, Some("BTC-EUR"), false).unwrap() else {
        panic!()
    };
    assert_eq!(get(&first[0], "samples"), Some(&Value::from(5565)));
    assert_eq!(get(&first[0], "already_seeded"), Some(&Value::Bool(false)));
    let Value::List(second) = ensure_bundled_seed(c, Some("BTC-EUR"), false).unwrap() else {
        panic!()
    };
    assert_eq!(get(&second[0], "samples"), Some(&Value::from(0)));
    assert_eq!(get(&second[0], "already_seeded"), Some(&Value::Bool(true)));
    assert_eq!(get(&second[0], "fetched_at"), Some(&Value::Null));
}
#[test]
fn retryable_statuses_remain_bounded_and_typed() {
    let (_r, b) = book();
    let c = b.connection();
    let server = kassiber_net::test_support::FakeProvider::replies(vec![
        (429, "{}".into()),
        (503, "{}".into()),
        (200, "{\"prices\":[]}".into()),
    ])
    .unwrap();
    let mut p = Providers::new(Gate::new(|| Offline::Online, Ledger::default()), "test");
    p.test_base = Some(server.base().into());
    let consent =
        FetchConsent::explicit(c, FetchAction::Latest, "coingecko", Some("BTC-EUR")).unwrap();
    p.latest(
        c,
        Some("BTC-EUR"),
        Some("coingecko"),
        Some(&consent),
        FetchAction::Latest,
    )
    .unwrap();
    assert_eq!(server.requests().len(), 3);
    let server = kassiber_net::test_support::FakeProvider::replies(vec![(
        429,
        "sensitive peer text".into(),
    )])
    .unwrap();
    p.test_base = Some(server.base().into());
    let error = p
        .latest(
            c,
            Some("BTC-EUR"),
            Some("coingecko"),
            Some(&consent),
            FetchAction::Latest,
        )
        .unwrap_err();
    assert_eq!(error.code, "rate_limited");
    assert_eq!(server.requests().len(), 3);
    assert!(!error.message.contains("sensitive"));
}
