//! The desktop exchange connection (Rust only): the explicit key check over
//! the signed loopback fakes, and the provider wallet a new connection gets.
use super::*;
use kassiber_net::{Ledger, Offline};
use kassiber_store::rusqlite::types::ValueRef;

fn gate(offline: Arc<AtomicBool>) -> Gate {
    Gate::new(
        move || {
            if offline.load(Ordering::SeqCst) {
                Offline::Offline
            } else {
                Offline::Online
            }
        },
        Ledger::default(),
    )
}

/// A fresh book as `loopback::run` prepares it: EUR and a BTC account.
fn book() -> Connection {
    let conn = crate::file_import::tests::fixture();
    conn.execute_batch("UPDATE profiles SET fiat_currency='EUR';
        INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','BTC','fixed');").unwrap();
    conn
}

/// One check on a fresh book; returns its result and every table's rows.
fn check(backend: &Json, consent: bool, offline: bool) -> (Result<Py, AppError>, Vec<String>) {
    let conn = book();
    let before = tables(&conn);
    let result = desktop::probe(
        &conn,
        backend,
        &gate(Arc::new(AtomicBool::new(offline))),
        "test",
        &mut |route| consent.then(|| Consent::explicit_trigger(route.request())),
    );
    // A check writes nothing, whatever it answers.
    assert_eq!(tables(&conn), before);
    (result, before)
}

/// Every row of every table, as text.
fn tables(conn: &Connection) -> Vec<String> {
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut rows = vec![];
    for name in names {
        let mut statement = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
        let width = statement.column_count();
        let mut found: Vec<String> = statement
            .query_map([], |r| {
                Ok((0..width)
                    .map(|i| match r.get_ref(i).unwrap() {
                        ValueRef::Text(text) => String::from_utf8_lossy(text).into_owned(),
                        other => format!("{other:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        found.sort();
        rows.extend(found.into_iter().map(|row| format!("{name}:{row}")));
    }
    rows
}

fn targets(fake: &loopback::Fake) -> Vec<String> {
    fake.seen
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.target.split('?').next().unwrap().to_owned())
        .collect()
}

const BINANCE_READ_ONLY: &str = r#"{"ipRestrict":false,"createTime":1700000000000,"enableReading":true,"enableWithdrawals":false,"enableInternalTransfer":false,"enableMargin":false,"enableFutures":false,"permitsUniversalTransfer":false,"enableVanillaOptions":false,"enableFixApiTrade":false,"enableFixReadOnly":true,"enableSpotAndMarginTrading":false,"enablePortfolioMarginTrading":false}"#;

fn binance(restrictions: &'static str) -> loopback::Fake {
    serve("binance", move |_, _, _| (200, restrictions.to_owned()))
}

#[test]
fn kraken_check_signs_the_first_page_of_both_histories() {
    let fake = serve("kraken", |o, _, t| kraken_page(o, t));
    let (result, _) = check(&backend("kraken", &fake.url), true, false);
    let result = result.unwrap();
    assert_eq!(at(&result, "exchange"), &s("kraken"));
    assert_eq!(at(&result, "ok"), &Py::Bool(true));
    // Kraken does not say what else a key may do.
    assert_eq!(at(&result, "read_only"), &Py::Null);
    let seen = fake.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|r| r.signed), "{seen:?}");
    assert_eq!(
        seen.iter()
            .map(|r| {
                let (path, body) = r.target.split_once('?').unwrap();
                format!("{path} {}", body.rsplit_once('&').unwrap().1)
            })
            .collect::<Vec<_>>(),
        ["/0/private/TradesHistory ofs=0", "/0/private/Ledgers ofs=0"]
    );
}

#[test]
fn kraken_check_reports_a_missing_permission() {
    let fake = serve("kraken", |_, _, _| {
        (200, r#"{"error":["EGeneral:Permission denied"]}"#.into())
    });
    let error = check(&backend("kraken", &fake.url), true, false)
        .0
        .unwrap_err();
    assert_eq!(error.code, "auth_error");
    assert_eq!(targets(&fake), ["/0/private/TradesHistory"]);
}

#[test]
fn coinbase_check_reads_one_accounts_page() {
    let fake = serve("coinbase", |_, _, _| {
        (
            200,
            r#"{"data":[{"id":"a","currency":{"code":"BTC"}}],"pagination":{"next_uri":"/v2/accounts?starting_after=a"}}"#.into(),
        )
    });
    let result = check(&backend("coinbase", &fake.url), true, false)
        .0
        .unwrap();
    assert_eq!(at(&result, "read_only"), &Py::Null);
    let seen = fake.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].signed);
    assert_eq!(seen[0].target, "/v2/accounts");
    let malformed = serve("coinbase", |_, _, _| (200, r#"{"data":{}}"#.into()));
    let error = check(&backend("coinbase", &malformed.url), true, false)
        .0
        .unwrap_err();
    assert_eq!(error.code, "protocol_error");
}

#[test]
fn binance_check_accepts_only_read_only_keys() {
    let fake = binance(BINANCE_READ_ONLY);
    let result = check(&backend("binance", &fake.url), true, false)
        .0
        .unwrap();
    assert_eq!(at(&result, "read_only"), &Py::Bool(true));
    let seen = fake.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].signed, "{seen:?}");
    assert_eq!(targets(&fake), ["/sapi/v1/account/apiRestrictions"]);

    for (restrictions, refused) in [
        (
            r#"{"enableReading":true,"enableWithdrawals":true,"enableSpotAndMarginTrading":true}"#,
            vec!["enableSpotAndMarginTrading", "enableWithdrawals"],
        ),
        (
            r#"{"enableReading":true,"permitsUniversalTransfer":true}"#,
            vec!["permitsUniversalTransfer"],
        ),
        // A flag Binance adds later counts as a write until Kassiber knows it.
        (
            r#"{"enableReading":true,"enableSomethingNew":true}"#,
            vec!["enableSomethingNew"],
        ),
    ] {
        let fake = binance(restrictions);
        let error = check(&backend("binance", &fake.url), true, false)
            .0
            .unwrap_err();
        assert_eq!(error.code, "auth_error", "{restrictions}");
        assert_eq!(
            error.details.as_ref().map(|d| at(d, "permissions").clone()),
            Some(Py::List(refused.into_iter().map(s).collect())),
            "{restrictions}"
        );
    }
    for restrictions in [
        r#"{"enableReading":false}"#,
        r#"{"enableReading":"true"}"#,
        r#"{}"#,
    ] {
        let error = check(&backend("binance", &binance(restrictions).url), true, false)
            .0
            .unwrap_err();
        assert_eq!(error.code, "auth_error", "{restrictions}");
        assert!(error.message.contains("cannot read"), "{restrictions}");
    }
    let error = check(&backend("binance", &binance("[]").url), true, false)
        .0
        .unwrap_err();
    assert_eq!(error.code, "protocol_error");
}

/// Bitpanda's documented shapes: one BTC crypto asset, EUR, one operation.
fn bitpanda(limits: (u16, &'static str)) -> loopback::Fake {
    serve("bitpanda", move |_, _, target| {
        match target.split('?').next().unwrap() {
            "/v1/trading-limits" => (limits.0, limits.1.to_owned()),
            "/v1/currencies" => (
                200,
                r#"{"data":[{"id":"b88b8466-efe3-11eb-b56f-0691764446a7","symbol":"EUR"}]}"#.into(),
            ),
            "/v1/operations" => (
                200,
                r#"{"data":[],"self_cursor":"","next_cursor":null,"has_next_page":false}"#.into(),
            ),
            _ => (404, r#"{"error":{"code":"not_found"}}"#.into()),
        }
    })
}

#[test]
fn bitpanda_check_refuses_trading_keys_then_reads_with_both_scopes() {
    let fake = bitpanda((401, r#"{"error":{"code":"insufficient_scope"}}"#));
    let result = check(&backend("bitpanda", &fake.url), true, false)
        .0
        .unwrap();
    assert_eq!(at(&result, "read_only"), &Py::Bool(true));
    let seen: Vec<String> = fake
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.target.clone())
        .collect();
    assert_eq!(
        seen,
        [
            "/v1/trading-limits",
            "/v1/currencies",
            "/v1/operations?page_size=1"
        ]
    );
    for limits in [
        (200, r#"{"data":[]}"#),
        (404, r#"{"error":{"code":"not_found"}}"#),
    ] {
        let fake = bitpanda(limits);
        let error = check(&backend("bitpanda", &fake.url), true, false)
            .0
            .unwrap_err();
        assert_eq!(error.code, "auth_error");
        assert_eq!(targets(&fake), ["/v1/trading-limits"]);
    }
}

#[test]
fn checks_need_consent_an_open_gate_and_credentials() {
    for kind in ["kraken", "coinbase", "binance", "bitpanda"] {
        let fake = serve(kind, |_, _, _| (500, "{}".into()));
        let refused = check(&backend(kind, &fake.url), false, false)
            .0
            .unwrap_err();
        assert_eq!(refused.code, "interaction_required", "{kind}");
        let offline = check(&backend(kind, &fake.url), true, true).0.unwrap_err();
        assert_eq!(offline.code, "network_egress_disabled", "{kind}");
        let mut missing = backend(kind, &fake.url);
        missing["token"] = json!(" ");
        let missing = check(&missing, true, false).0.unwrap_err();
        assert_eq!(missing.code, "config_error", "{kind}");
        assert!(fake.seen.lock().unwrap().is_empty(), "{kind}");
    }
    // Only an exchange backend is checked.
    let mut esplora = backend("kraken", "http://127.0.0.1:9");
    esplora["kind"] = json!("esplora");
    assert_eq!(
        check(&esplora, true, false).0.unwrap_err().code,
        "validation"
    );
}

#[test]
fn checks_withhold_an_echoed_key_and_secret() {
    for kind in ["kraken", "coinbase", "binance", "bitpanda"] {
        let stored = backend(kind, "http://127.0.0.1:9");
        // The secret as the backend stores it (Kraken's is base64).
        // Bitpanda has none: it authenticates with the key alone.
        let secret = stored["auth_header"]
            .as_str()
            .unwrap_or(loopback::KEY)
            .to_owned();
        let echoed = secret.clone();
        let fake = serve(kind, move |_, _, _| {
            (
                401,
                json!({"error": {"code": format!("bad {} {echoed}", loopback::KEY)}, "msg": loopback::KEY})
                    .to_string(),
            )
        });
        let error = check(&backend(kind, &fake.url), true, false).0.unwrap_err();
        let shown = kassiber_pyjson::dumps(&error.envelope(None), &Default::default()).unwrap();
        assert!(!shown.contains(loopback::KEY), "{kind}: {shown}");
        assert!(!shown.contains(&secret), "{kind}: {shown}");
        assert!(shown.contains("<redacted>"), "{kind}: {shown}");
    }
}

#[test]
fn a_new_connection_wallet_is_the_one_the_cli_sync_creates() {
    let identity = Identity {
        ids: vec!["00000000-0000-4000-8000-0000000000aa".into()],
        timestamp: "2026-11-08T12:00:00Z".into(),
    };
    // The CLI: `wallets sync-kraken` creates the Kraken wallet on its first
    // run (its first identity), then imports.
    let fake = serve("kraken", |o, _, t| kraken_page(o, t));
    let cli = book();
    let cli_ids = Identity {
        ids: (0..16)
            .map(|n| format!("00000000-0000-4000-8000-{:012x}", 0xaa + n))
            .collect(),
        timestamp: identity.timestamp.clone(),
    };
    let gate = gate(online());
    execute(
        &cli,
        "p",
        "",
        &backend("kraken", &fake.url),
        "kraken",
        &gate,
        "test",
        &mut |route| Some(Consent::explicit_trigger(route.request())),
        Some(&cli_ids),
    )
    .unwrap();
    // The desktop: the connection creates that wallet, then its sync imports
    // with the identities that follow.
    let desktop_conn = book();
    let wallet = desktop::create_wallet(
        &desktop_conn,
        "p",
        "Kraken",
        "kraken",
        "fixture",
        Some(&identity),
    )
    .unwrap();
    assert_eq!(wallet.id, identity.ids[0]);
    let rest = Identity {
        ids: cli_ids.ids[1..].to_vec(),
        timestamp: identity.timestamp.clone(),
    };
    execute(
        &desktop_conn,
        "p",
        &wallet.id,
        &backend("kraken", &fake.url),
        "kraken",
        &gate,
        "test",
        &mut |route| Some(Consent::explicit_trigger(route.request())),
        Some(&rest),
    )
    .unwrap();
    // Each fixture stamps its own creation time on the migration audits.
    let rows = |conn: &Connection| -> Vec<String> {
        tables(conn)
            .into_iter()
            .filter(|row| !row.starts_with("schema_migration_audits:"))
            .collect()
    };
    let (desktop_rows, cli_rows) = (rows(&desktop_conn), rows(&cli));
    assert!(cli_rows.iter().any(|row| row.starts_with("transactions:")));
    let only = |a: &[String], b: &[String]| -> Vec<String> {
        a.iter().filter(|row| !b.contains(row)).cloned().collect()
    };
    assert_eq!(
        (
            only(&desktop_rows, &cli_rows),
            only(&cli_rows, &desktop_rows)
        ),
        (vec![], vec![])
    );
    assert_eq!(
        count(
            &cli,
            "SELECT count(*) FROM wallets WHERE label='Kraken' AND config_json='{\"backend\": \"fixture\"}'"
        ),
        1
    );
}

#[test]
fn sync_targets_are_the_profiles_exchange_wallets() {
    let conn = book();
    for (kind, label) in [("bitpanda", "Bitpanda"), ("kraken", "Kraken 2")] {
        desktop::create_wallet(&conn, "p", label, kind, &format!("{kind}-b"), None).unwrap();
    }
    let listed: Vec<_> = desktop::targets(&conn, "p")
        .unwrap()
        .into_iter()
        .map(|t| (t.label, t.kind, t.backend))
        .collect();
    assert_eq!(
        listed,
        [
            (
                "Bitpanda".into(),
                "bitpanda".into(),
                Some("bitpanda-b".into())
            ),
            ("Kraken 2".into(), "kraken".into(), Some("kraken-b".into())),
        ]
    );
    assert!(desktop::targets(&conn, "q").unwrap().is_empty());
    let target = desktop::sync_target(&conn, "p", "kraken 2").unwrap();
    assert_eq!(
        (target.kind.as_str(), target.label.as_str()),
        ("kraken", "Kraken 2")
    );
    // The fixture's custom wallet is not an exchange connection.
    assert_eq!(
        desktop::sync_target(&conn, "p", "Source")
            .err()
            .unwrap()
            .code,
        "validation"
    );
    assert_eq!(
        desktop::create_wallet(&conn, "p", "X", "esplora", "b", None)
            .err()
            .unwrap()
            .code,
        "validation"
    );
}
