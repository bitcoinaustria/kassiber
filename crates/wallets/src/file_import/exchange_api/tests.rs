//! Local codec and conversion vectors computed with CPython 3.11 and the
//! Python reference helpers. Wire and book parity lives in
//! tests/test_rust_exchange_api.py.
use super::*;

#[test]
fn kraken_secret_decoding_follows_non_validating_binascii() {
    for (text, expected) in [
        ("c3ludGhldGlj", Some("73796e746865746963")),
        ("!!c3ludGhldGlj", Some("73796e746865746963")),
        ("c3ludGhldGljLQ==", Some("73796e7468657469632d")),
        ("c3ludGhldGljLQ=x=", Some("73796e7468657469632d0c")),
        ("ABC", None),
        ("A===", None),
        ("AB==CD", Some("00")),
        ("", Some("")),
        ("YWJj\nZGVm", Some("616263646566")),
        ("QQ", None),
        ("QUI=", Some("4142")),
        ("é", None),
    ] {
        let decoded = kraken::base64_decode(text)
            .map(|bytes| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
        assert_eq!(decoded.as_deref(), expected, "{text:?}");
    }
}

#[test]
fn utc_timestamps_round_half_even_and_report_cpython_range_errors() {
    for (seconds, expected) in [
        (0.0, "1970-01-01T00:00:00Z"),
        (1_700_000_150.999_999_5, "2023-11-14T22:15:51Z"),
        (-0.5, "1969-12-31T23:59:59Z"),
        (-1e-7, "1970-01-01T00:00:00Z"),
        (1.5e-7, "1970-01-01T00:00:00Z"),
        (-62_135_596_800.0, "0001-01-01T00:00:00Z"),
        (253_402_300_800.0, "year 10000 is out of range"),
        (-62_135_596_801.0, "year 0 is out of range"),
        (6.7e16, "year 2123147449 is out of range"),
        (6.8e16, "[Errno 75] Value too large for defined data type"),
        (1e17, "[Errno 75] Value too large for defined data type"),
        (9.3e18, "timestamp out of range for platform time_t"),
        (f64::NAN, "Invalid value NaN (not a number)"),
        (f64::INFINITY, "timestamp out of range for platform time_t"),
        (
            f64::NEG_INFINITY,
            "timestamp out of range for platform time_t",
        ),
    ] {
        let actual = utc_iso(seconds).unwrap_or_else(|e| e.message);
        assert_eq!(actual, expected, "{seconds}");
    }
}

#[test]
fn assets_follow_normalize_asset_code_and_exchange_aliases() {
    for (raw, expected) in [
        (s("xbt"), "BTC"),
        (s(" XXBT "), "BTC"),
        (s("l-btc"), "LBTC"),
        (s("L-BTC"), "LBTC"),
        (s("zeur"), "EUR"),
        (s("XETH"), "ETH"),
        (s("xeth"), "ETH"),
        (s("uſd"), "USD"),
        (s("BTCs"), "BTCS"),
        (s("Zé12"), "É12"),
        (s(""), ""),
        (Py::Null, ""),
        (Py::from(0), ""),
    ] {
        assert_eq!(asset(&raw), expected, "{raw:?}");
    }
    assert_eq!(asset(&s("A".repeat(64))), "a".repeat(64));
}

#[test]
fn kraken_pairs_prefer_base_prefixes_then_fiat_suffixes() {
    for (pair, base, expected) in [
        ("XXBTZEUR", "BTC", Some("ZEUR")),
        ("XBT/USD", "BTC", Some("USD")),
        ("wbtc/zeur", "BTC", Some("ZEUR")),
        ("XBTETH", "BTC", Some("ETH")),
        ("XXBT", "BTC", None),
        ("LBTCEUR", "LBTC", Some("EUR")),
        ("FOOCHF", "BTC", Some("CHF")),
        ("FOO", "BTC", None),
    ] {
        assert_eq!(kraken::quote(pair, base).as_deref(), expected, "{pair}");
    }
}

#[test]
fn signatures_match_the_providers_reference_construction() {
    let digest = sha256::Hash::hash(b"123nonce=123&ofs=0");
    assert_eq!(
        kassiber_chain::encode_base64(&hmac_sha512(
            b"secret",
            &[b"/0/private/Ledgers", digest.as_byte_array()]
        )),
        "DWUwXqtMAj/kzFfS0GI2nskeqylKsg6ZTLB6uIFyEeXedLjJrTUujQ32kd4GNjCUiOysCGULoiDAytuw6IprDg=="
    );
    assert_eq!(
        hmac_sha256_hex(b"secret", b"1700000000GET/v2/accounts"),
        "0048a8d4062b25010ccfc40824a351ef2882528ef0a436d6c20c10ffc6ae8377"
    );
}

#[test]
fn coinbase_ids_quote_like_urllib_and_pages_stay_in_their_collection() {
    for (raw, expected) in [
        ("btc-wallet", "btc-wallet"),
        ("l btc/€", "l%20btc/%E2%82%AC"),
        ("a~b_c.d-e", "a~b_c.d-e"),
        ("?#%", "%3F%23%25"),
    ] {
        assert_eq!(coinbase::quote(&raw.into()).unwrap(), expected);
    }
    let base = "https://api.coinbase.com";
    let collection = "/v2/accounts/a/transactions";
    assert_eq!(
        coinbase::page_url(
            base,
            collection,
            "/v2/accounts/a/transactions?starting_after=x"
        )
        .unwrap(),
        "https://api.coinbase.com/v2/accounts/a/transactions?starting_after=x"
    );
    for hostile in [
        "/v2/accounts",
        "/v2/accounts/a/transactions/../../b",
        "/v2/accounts/a/transactionsx",
        "/v2/accounts/a/transactions?x=1#frag",
        "/v2/accounts/a/transactions?x=a b",
        "/v2/accounts/a/transactions?x=\u{e9}",
        "//evil.example/v2/accounts/a/transactions",
        "https://evil.example/v2/accounts/a/transactions",
    ] {
        assert_eq!(
            coinbase::page_url(base, collection, hostile)
                .unwrap_err()
                .code,
            "protocol_error",
            "{hostile}"
        );
    }
    // A base with its own path keeps it, and the signed path is what is sent.
    assert_eq!(
        coinbase::page_url(
            "http://127.0.0.1:1/api",
            "/v2/accounts",
            "/v2/accounts?x=%41"
        )
        .unwrap(),
        "http://127.0.0.1:1/api/v2/accounts?x=%41"
    );
}

#[test]
fn decimals_accept_python_syntax_and_report_python_text() {
    for (raw, expected) in [
        (s(" 0.1 "), "0.1"),
        (s("1_000"), "1000"),
        (s("in_f"), "Infinity"),
        (s("NaN12"), "NaN12"),
        (s("-inf"), "-Infinity"),
        (Py::Float(1e300), "1E+300"),
        (Py::Null, "0"),
        (s(""), "0"),
    ] {
        assert_eq!(repr(&decimal(&raw, "0").unwrap()), expected, "{raw:?}");
    }
    for (raw, message) in [
        (Py::Bool(true), "Invalid decimal value: True"),
        (Py::List(vec![]), "Invalid decimal value: []"),
    ] {
        assert_eq!(decimal(&raw, "0").err().unwrap().message, message);
    }
}

#[test]
fn division_by_an_infinity_is_a_signed_subnormal_zero() {
    let one = PriceNumber::parse("1").unwrap();
    let infinity = PriceNumber::parse("-Infinity").unwrap();
    assert_eq!(repr(&divide(&one, &infinity).unwrap()), "-0E-1000030");
    let zero = PriceNumber::parse("0").unwrap();
    let amount = PriceNumber::parse("-0.0003").unwrap();
    assert_eq!(repr(&divide(&zero, &amount).unwrap()), "-0E+4");
}

/// Every Python normalizer vector, replayed through the Rust normalizer that
/// receives the same assembled payload. `raw_json` is compared by digest.
#[test]
fn normalizers_match_python_reference_vectors() {
    let vectors = kassiber_pytext::loads(include_str!(
        "../../../tests/vectors/python_exchange_api.json"
    ))
    .unwrap();
    let Py::List(vectors) = vectors else {
        panic!("vector list")
    };
    assert!(vectors.len() > 4000);
    let mut failures = vec![];
    for vector in &vectors {
        let exchange = kassiber_envelope::py_str(at(vector, "exchange"));
        let name = kassiber_envelope::py_str(at(vector, "name"));
        let payload = at(vector, "payload");
        let result = match exchange.as_str() {
            "kraken" => kraken::normalize(at(payload, "ledgers"), at(payload, "trades")),
            "coinbase" => {
                let Py::List(accounts) = payload else {
                    panic!("accounts")
                };
                let accounts: Vec<(PyStr, Vec<Py>)> = accounts
                    .iter()
                    .map(|account| {
                        let Py::List(pair) = account else {
                            panic!("pair")
                        };
                        let Py::List(rows) = &pair[1] else {
                            panic!("rows")
                        };
                        (text(&pair[0]), rows.clone())
                    })
                    .collect();
                coinbase::records(&accounts)
            }
            _ => binance::normalize_payload(payload),
        };
        let actual = match result {
            Ok(records) => dict([(
                "records",
                Py::List(
                    records
                        .into_iter()
                        .map(|mut record| {
                            let raw = kassiber_envelope::py_str(at(&record, "raw_json"));
                            let digest = sha256::Hash::hash(raw.as_bytes()).to_string();
                            kassiber_envelope::set(&mut record, "raw_json", s(&digest[..16]));
                            record
                        })
                        .collect(),
                ),
            )]),
            Err(error) => dict([(
                "error",
                dict([
                    ("code", s(error.code.as_str())),
                    (
                        "message",
                        error
                            .message_codepoints
                            .as_deref()
                            .map_or_else(|| s(error.message.as_str()), |m| Py::Str(m.clone())),
                    ),
                    ("hint", error.hint.as_deref().map_or(Py::Null, s)),
                    ("details", error.details.clone().unwrap_or(Py::Null)),
                    ("retryable", Py::Bool(error.retryable)),
                ]),
            )]),
        };
        let dump = |value: &Py| kassiber_pyjson::dumps(value, &Default::default()).unwrap();
        if dump(&actual) != dump(at(vector, "expected")) {
            failures.push(format!(
                "{exchange} {name}\n  rust:   {}\n  python: {}",
                dump(&actual),
                dump(at(vector, "expected"))
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} vectors differ:\n{}",
        failures.len(),
        vectors.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

mod loopback {
    //! A signed loopback exchange: each provider's signature is checked the
    //! way the provider checks it, and every request is recorded.
    use super::*;
    use kassiber_net::test_support::HandledProvider;
    use kassiber_net::{Ledger, Offline};
    use std::sync::{Arc, Mutex};

    pub const KEY: &str = "synthetic-exchange-key";
    pub const SECRET: &str = "synthetic-exchange-secret";

    #[derive(Clone, Debug)]
    pub struct Seen {
        pub target: String,
        pub signed: bool,
    }

    pub struct Fake {
        pub url: String,
        pub seen: Arc<Mutex<Vec<Seen>>>,
        // Serves until the fake is dropped.
        _server: HandledProvider,
    }

    fn signed(
        kind: &str,
        method: &str,
        target: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> bool {
        let header = |name: &str| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
                .unwrap_or("")
        };
        match kind {
            "kraken" => {
                let text = std::str::from_utf8(body).unwrap();
                let nonce = text
                    .split('&')
                    .find_map(|p| p.strip_prefix("nonce="))
                    .unwrap_or("");
                let digest = sha256::Hash::hash(format!("{nonce}{text}").as_bytes());
                let expected = kassiber_chain::encode_base64(&hmac_sha512(
                    SECRET.as_bytes(),
                    &[target.as_bytes(), digest.as_byte_array()],
                ));
                method == "POST" && header("API-Key") == KEY && header("API-Sign") == expected
            }
            "coinbase" => {
                let stamp = header("CB-ACCESS-TIMESTAMP");
                let expected =
                    hmac_sha256_hex(SECRET.as_bytes(), format!("{stamp}GET{target}").as_bytes());
                method == "GET"
                    && header("CB-ACCESS-KEY") == KEY
                    && header("CB-VERSION") == "2017-11-27"
                    && header("CB-ACCESS-SIGN") == expected
            }
            // Bitpanda reads carry the key alone, and only GET is ever sent.
            "bitpanda" => method == "GET" && header("x-api-key") == KEY && body.is_empty(),
            _ => {
                let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
                let Some((unsigned, signature)) = query.rsplit_once("&signature=") else {
                    return false;
                };
                method == "GET"
                    && header("X-MBX-APIKEY") == KEY
                    && signature == hmac_sha256_hex(SECRET.as_bytes(), unsigned.as_bytes())
            }
        }
    }

    /// `reply(ordinal, method, target)` answers each signed request; an
    /// unsigned one gets 401, as the providers answer it.
    pub fn serve(
        kind: &'static str,
        reply: impl Fn(usize, &str, &str) -> (u16, String) + Send + 'static,
    ) -> Fake {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let server = HandledProvider::start(move |request| {
            let (method, target) = (request.method.as_str(), request.target.as_str());
            let path = target.split('?').next().unwrap();
            let is_signed = signed(
                kind,
                method,
                if kind == "kraken" { path } else { target },
                &request.headers,
                &request.body,
            );
            // A form body is shown as the query it encodes.
            let shown = if method == "POST" {
                format!("{target}?{}", String::from_utf8_lossy(&request.body))
            } else {
                target.to_owned()
            };
            let ordinal = {
                let mut log = log.lock().unwrap();
                log.push(Seen {
                    target: shown.clone(),
                    signed: is_signed,
                });
                log.len() - 1
            };
            if is_signed {
                reply(ordinal, method, &shown)
            } else {
                (401, "{\"error\":[\"EAPI:Invalid key\"]}".into())
            }
        })
        .unwrap();
        Fake {
            url: server.base().to_owned(),
            seen,
            _server: server,
        }
    }

    pub fn backend(kind: &str, url: &str) -> Json {
        if kind == "bitpanda" {
            return json!({"name": "fixture", "kind": kind, "url": url, "token": KEY, "timeout": 5});
        }
        let secret = if kind == "kraken" {
            kassiber_chain::encode_base64(SECRET.as_bytes())
        } else {
            SECRET.to_owned()
        };
        json!({"name": "fixture", "kind": kind, "url": url, "token": KEY, "auth_header": secret, "timeout": 5})
    }

    pub struct Run {
        pub result: Result<Py, AppError>,
        pub conn: Connection,
    }

    /// One `execute` on a fresh book. `offline` flips the gate once set.
    pub fn run(
        backend: &Json,
        expected: &str,
        wallet: &str,
        consent: bool,
        offline: Arc<AtomicBool>,
    ) -> Run {
        run_prepared(backend, expected, wallet, consent, offline, &|_| ())
    }

    /// As `run`, after `prepare` has seen the fresh book.
    pub fn run_prepared(
        backend: &Json,
        expected: &str,
        wallet: &str,
        consent: bool,
        offline: Arc<AtomicBool>,
        prepare: &dyn Fn(&Connection),
    ) -> Run {
        let conn = crate::file_import::tests::fixture();
        conn.execute_batch("UPDATE profiles SET fiat_currency='EUR';
            INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','BTC','fixed');").unwrap();
        prepare(&conn);
        let flag = offline.clone();
        let gate = Gate::new(
            move || {
                if flag.load(Ordering::SeqCst) {
                    Offline::Offline
                } else {
                    Offline::Online
                }
            },
            Ledger::default(),
        );
        let result = execute(
            &conn,
            "p",
            wallet,
            backend,
            expected,
            &gate,
            "test",
            &mut |route| consent.then(|| Consent::explicit_trigger(route.request())),
            None,
        );
        Run { result, conn }
    }
}

mod bitpanda_api;
mod desktop_api;

use loopback::{backend, run, serve};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn online() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn kraken_page(ordinal: usize, target: &str) -> (u16, String) {
    kraken_sized(ordinal, target, 2)
}
fn kraken_sized(ordinal: usize, target: &str, size: usize) -> (u16, String) {
    let _ = ordinal;
    let offset: usize = target
        .rsplit("ofs=")
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let (field, rows) = if target.starts_with("/0/private/TradesHistory") {
        (
            "trades",
            vec![
                (
                    "T1",
                    r#"{"pair":"XXBTZEUR","cost":"400.00","fee":"1.20","price":"40000.00"}"#,
                ),
                ("T2", r#"{"pair":"XBTEUR","cost":"42.00","fee":"0.26"}"#),
            ],
        )
    } else {
        (
            "ledger",
            vec![
                (
                    "L1",
                    r#"{"refid":"T1","time":1700000000.125,"type":"trade","asset":"XXBT","amount":"0.01","fee":"0"}"#,
                ),
                (
                    "L2",
                    r#"{"refid":"T2","time":"1700000100","type":"trade","asset":"XBT","amount":"-0.001","fee":"0.000001"}"#,
                ),
                (
                    "L3",
                    r#"{"time":"1700000200","type":"withdrawal","asset":"BTC","amount":"-0.002","fee":"0.00001"}"#,
                ),
            ],
        )
    };
    let page = rows
        .iter()
        .skip(offset)
        .take(size)
        .map(|(id, row)| format!("\"{id}\":{row}"))
        .collect::<Vec<_>>()
        .join(",");
    (
        200,
        format!(
            "{{\"error\":[],\"result\":{{\"count\":{},\"{field}\":{{{page}}}}}}}",
            rows.len()
        ),
    )
}

#[test]
fn kraken_signs_every_page_and_imports_through_the_coordinator() {
    let fake = serve("kraken", |ordinal, _, target| kraken_page(ordinal, target));
    let run = run(&backend("kraken", &fake.url), "kraken", "", true, online());
    let result = run.result.unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(3), "{result:?}");
    assert_eq!(at(&result, "fetched"), &Py::from(3));
    assert_eq!(at(&result, "source"), &s("kraken:api:fixture"));
    let seen = fake.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|r| r.signed), "{seen:?}");
    let pages: Vec<_> = seen
        .iter()
        .map(|r| {
            let (path, body) = r.target.split_once('?').unwrap();
            (path.to_owned(), body.rsplit_once('&').unwrap().1.to_owned())
        })
        .collect();
    assert_eq!(
        pages,
        [
            ("/0/private/TradesHistory", "ofs=0"),
            ("/0/private/Ledgers", "ofs=0"),
            ("/0/private/Ledgers", "ofs=2"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    // A sell without a quoted price is priced at Kraken's cost / volume.
    let rate: String = run
        .conn
        .query_row(
            "SELECT fiat_rate FROM transactions WHERE external_id='kraken:L2'",
            [],
            |r| {
                r.get::<_, Option<f64>>(0)
                    .map(|v| v.unwrap_or_default().to_string())
            },
        )
        .unwrap();
    assert_eq!(rate, "42000");
    assert_eq!(
        count(
            &run.conn,
            "SELECT count(*) FROM wallets WHERE label='Kraken' AND kind='kraken'"
        ),
        1
    );
}

#[test]
fn kraken_unparsable_counts_stop_after_the_merged_rows() {
    let fake = serve("kraken", |_, _, target| {
        let (status, body) = kraken_page(0, target);
        (
            status,
            body.replace("\"count\":3", "\"count\":\"many\"")
                .replace("\"count\":2", "\"count\":null"),
        )
    });
    let run = run(&backend("kraken", &fake.url), "kraken", "", true, online());
    assert_eq!(at(&run.result.unwrap(), "fetched"), &Py::from(2));
    assert_eq!(fake.seen.lock().unwrap().len(), 2);
}

#[test]
fn kraken_errors_keep_permission_codes_and_withhold_credentials() {
    let fake = serve("kraken", |_, _, _| {
        (
            200,
            format!(
                "{{\"error\":[\"EGeneral:Permission denied for {}\"]}}",
                loopback::KEY
            ),
        )
    });
    let error = run(&backend("kraken", &fake.url), "kraken", "", true, online())
        .result
        .unwrap_err();
    assert_eq!(error.code, "auth_error");
    let details =
        kassiber_pyjson::dumps(error.details.as_ref().unwrap(), &Default::default()).unwrap();
    assert!(!details.contains(loopback::KEY), "{details}");
    assert!(
        details.contains("Permission denied for <redacted>"),
        "{details}"
    );
}

#[test]
fn exchange_reads_need_the_callers_consent_and_an_open_gate() {
    let fake = serve("kraken", |_, _, target| kraken_page(0, target));
    let refused = run(&backend("kraken", &fake.url), "kraken", "", false, online());
    assert_eq!(refused.result.unwrap_err().code, "interaction_required");
    // Python creates and commits the provider wallet before any request.
    assert_eq!(
        count(
            &refused.conn,
            "SELECT count(*) FROM wallets WHERE label='Kraken'"
        ),
        1
    );
    let offline = Arc::new(AtomicBool::new(true));
    let blocked = run(&backend("kraken", &fake.url), "kraken", "", true, offline);
    assert!(blocked.result.is_err());
    assert!(fake.seen.lock().unwrap().is_empty());
    let mut missing = backend("kraken", &fake.url);
    missing["token"] = json!("  ");
    let missing = run(&missing, "kraken", "", true, online());
    assert_eq!(missing.result.unwrap_err().code, "config_error");
    assert!(fake.seen.lock().unwrap().is_empty());
}

#[test]
fn going_offline_after_the_reads_publishes_nothing() {
    use kassiber_store::rusqlite::functions::FunctionFlags;
    // The book goes offline from inside the import itself, after every read
    // has been answered: only the recheck before publication can notice.
    let fake = serve("kraken", |o, _, t| kraken_page(o, t));
    let offline = online();
    let flag = offline.clone();
    let run = loopback::run_prepared(
        &backend("kraken", &fake.url),
        "kraken",
        "Source",
        true,
        offline,
        &move |conn| {
            let flag = flag.clone();
            conn.create_scalar_function("go_offline", 0, FunctionFlags::SQLITE_UTF8, move |_| {
                flag.store(true, Ordering::SeqCst);
                Ok(0)
            })
            .unwrap();
            conn.execute_batch(
                "CREATE TEMP TRIGGER offline_on_insert AFTER INSERT ON transactions BEGIN SELECT go_offline(); END;",
            )
            .unwrap();
        },
    );
    assert_eq!(fake.seen.lock().unwrap().len(), 3);
    assert_eq!(run.result.unwrap_err().code, "network_egress_disabled");
    assert_eq!(count(&run.conn, "SELECT count(*) FROM transactions"), 0);
}

#[test]
fn coinbase_signs_pages_follows_its_collection_and_keeps_coinbase_network_metadata() {
    let fake = serve("coinbase", |_, _, target| {
        let body = match target {
            "/v2/accounts" => {
                r#"{"data":[{"id":"btc 1","currency":{"code":"BTC"}},{"id":"eur","currency":{"code":"EUR"}}],"pagination":{"next_uri":"/v2/accounts?starting_after=eur"}}"#
            }
            "/v2/accounts?starting_after=eur" => {
                r#"{"data":[{"id":"","currency":"BTC"}],"pagination":{}}"#
            }
            "/v2/accounts/btc%201/transactions" => {
                r#"{"data":[{"id":"b1","type":"buy","created_at":"2024-01-02T03:04:05Z","amount":{"amount":"0.01","currency":"BTC"},"native_amount":{"amount":"-400","currency":"EUR"},"buy":{"commission":"1.5"}}],"pagination":{"next_uri":"/v2/accounts/btc%201/transactions?starting_after=b1"}}"#
            }
            _ => {
                r#"{"data":[{"id":"s1","type":"send","created_at":"2024-01-03T03:04:05Z","amount":{"amount":"-0.001","currency":"BTC"},"network":{"hash":"tx-hash","status":"confirmed"}},{"id":"x","type":"sell","created_at":"2024-01-04T00:00:00Z","amount":{"amount":"-0.002","currency":"BTC"},"native_amount":{"amount":"-80","currency":"EUR"},"sell":{"commission":"2"}}],"pagination":{"next_uri":null}}"#
            }
        };
        (200, body.into())
    });
    let run = run(
        &backend("coinbase", &fake.url),
        "coinbase",
        "",
        true,
        online(),
    );
    let result = run.result.unwrap();
    assert_eq!(at(&result, "imported"), &Py::from(3));
    let seen = fake.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|r| r.signed), "{seen:?}");
    assert_eq!(seen.len(), 4);
    let value = |id: &str| -> String {
        run.conn
            .query_row(
                "SELECT CAST(fiat_value_exact AS TEXT) FROM transactions WHERE external_id=?",
                [id],
                |r| r.get::<_, Option<String>>(0),
            )
            .unwrap()
            .unwrap_or_default()
    };
    // Commission is added to a buy's cost only; a send keys on its hash.
    assert!(
        value("coinbase:b1").starts_with("401.5"),
        "{}",
        value("coinbase:b1")
    );
    assert!(
        value("coinbase:x").starts_with("80"),
        "{}",
        value("coinbase:x")
    );
    assert_eq!(
        count(
            &run.conn,
            "SELECT count(*) FROM transactions WHERE external_id='tx-hash'"
        ),
        1
    );
}

#[test]
fn binance_signs_each_history_query_in_reference_order() {
    let fake = serve("binance", |_, _, target| {
        let path = target.split('?').next().unwrap();
        let body = match path {
            "/sapi/v1/fiat/payments" => {
                r#"{"data":[{"orderNo":"F1","status":"Completed","cryptoCurrency":"BTC","fiatCurrency":"EUR","obtainAmount":"0.01","sourceAmount":"-400.5","createTime":1700000000123}]}"#
            }
            "/sapi/v1/capital/deposit/hisrec" => {
                r#"[{"coin":"BTC","status":"Confirming","amount":"0.005","insertTime":1700000100000,"txId":"d1"}]"#
            }
            "/sapi/v1/capital/withdraw/history" => {
                r#"[{"coin":"BTC","status":6,"amount":"0.004","transactionFee":"0.0001","applyTime":1700000500000,"id":"w-1"}]"#
            }
            _ => {
                r#"{"rows":[{"id":7,"asset":"BTC","amount":"0.0000123","divTime":1700000700000,"enInfo":"BTC Mining Pool"}]}"#
            }
        };
        (200, body.into())
    });
    let run = run(
        &backend("binance", &fake.url),
        "binance",
        "",
        true,
        online(),
    );
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(4));
    let seen = fake.seen.lock().unwrap().clone();
    assert!(seen.iter().all(|r| r.signed), "{seen:?}");
    let paths: Vec<_> = seen
        .iter()
        .map(|r| r.target.split('?').next().unwrap().to_owned())
        .collect();
    assert_eq!(
        paths,
        [
            "/sapi/v1/fiat/payments",
            "/sapi/v1/capital/deposit/hisrec",
            "/sapi/v1/capital/withdraw/history",
            "/sapi/v1/asset/assetDividend"
        ]
    );
    assert!(seen[0]
        .target
        .contains("transactionType=0&beginTime=1498867200000&endTime="));
    assert_eq!(
        count(
            &run.conn,
            "SELECT count(*) FROM transactions WHERE kind='mining'"
        ),
        1
    );
}

/// One Binance sync whose deposit and withdrawal name `rail`.
fn binance_rail_run(rail: &'static str) -> loopback::Run {
    let fake = serve("binance", move |_, _, target| {
        let body = match target.split('?').next().unwrap() {
            "/sapi/v1/capital/deposit/hisrec" => format!(
                r#"[{{"coin":"BTC","network":"{rail}","status":1,"amount":"0.005","insertTime":1700000100000,"txId":"d1"}}]"#
            ),
            "/sapi/v1/capital/withdraw/history" => format!(
                r#"[{{"coin":"BTC","network":"{rail}","status":6,"amount":"0.004","transactionFee":"0.0001","applyTime":1700000500000,"id":"w-1"}}]"#
            ),
            "/sapi/v1/fiat/payments" => r#"{"data":[]}"#.to_owned(),
            _ => r#"{"rows":[]}"#.to_owned(),
        };
        (200, body)
    });
    run(
        &backend("binance", &fake.url),
        "binance",
        "",
        true,
        online(),
    )
}

#[test]
fn binance_btc_rail_imports_and_other_rails_refuse() {
    // Binance names the coin's transfer rail: "BTC" is Bitcoin on chain.
    let run = binance_rail_run("BTC");
    assert_eq!(at(&run.result.unwrap(), "imported"), &Py::from(2));
    assert_eq!(
        count(
            &run.conn,
            "SELECT count(*) FROM transactions WHERE json_extract(raw_json,'$.network')='BTC'"
        ),
        2
    );
    for rail in ["LIGHTNING", "BSC", "btc"] {
        let run = binance_rail_run(rail);
        assert_eq!(
            run.result.unwrap_err().code,
            "book_network_mismatch",
            "{rail}"
        );
        assert_eq!(
            count(&run.conn, "SELECT count(*) FROM transactions"),
            0,
            "{rail}"
        );
    }
}

#[test]
fn non_finite_amounts_report_before_the_rows_price_currency_check() {
    let fake = serve("kraken", |ordinal, _, target| {
        let (status, body) = kraken_page(ordinal, target);
        (
            status,
            body.replace("\"amount\":\"0.01\"", "\"amount\":\"Infinity\"")
                .replace("XXBTZEUR", "XXBTZUSD"),
        )
    });
    let error = run(
        &backend("kraken", &fake.url),
        "kraken",
        "Source",
        true,
        online(),
    )
    .result
    .unwrap_err();
    assert_eq!(
        error.message, "cannot convert Infinity to integer",
        "{error:?}"
    );
}

#[test]
fn echoed_credentials_are_withheld_in_every_spelling_python_withholds() {
    // Computed with kassiber.redaction.withhold_credentials.
    let secret = "a b/+\u{e9}\"\\";
    let text = "x a b/+\u{e9}\"\\ | a%20b%2F%2B%C3%A9%22%5C | a+b%2F%2B%C3%A9%22%5C | a b/+\\u00e9\\\"\\\\ | a b\\/+\\u00e9\\\"\\\\ | key";
    assert_eq!(
        kassiber_chain::withhold_credentials(text, &[secret, "key"]),
        "x <redacted> | <redacted> | <redacted> | <redacted> | <redacted> | <redacted>"
    );
}

#[test]
fn a_kraken_error_string_is_one_item_with_its_echoed_key_withheld() {
    let fake = serve("kraken", |_, _, _| {
        (
            200,
            format!("{{\"error\":\"EAPI:Invalid key {}\"}}", loopback::KEY),
        )
    });
    let error = run(&backend("kraken", &fake.url), "kraken", "", true, online())
        .result
        .unwrap_err();
    assert_eq!(error.code, "protocol_error");
    let details =
        kassiber_pyjson::dumps(error.details.as_ref().unwrap(), &Default::default()).unwrap();
    assert_eq!(
        details,
        r#"{"kraken_errors": ["EAPI:Invalid key <redacted>"]}"#
    );
}

#[test]
fn http_error_bodies_withhold_credentials_before_truncation() {
    // The secret straddles the 200-character cut: withheld first, it cannot
    // leave a prefix behind.
    let fake = serve("coinbase", |_, _, _| {
        (
            400,
            format!("{}{}{}", "x".repeat(195), loopback::SECRET, "%22"),
        )
    });
    let error = run(
        &backend("coinbase", &fake.url),
        "coinbase",
        "",
        true,
        online(),
    )
    .result
    .unwrap_err();
    assert!(error
        .message
        .starts_with("HTTP 400 from backend for http://<redacted-host>: "));
    assert!(error.message.ends_with("x<reda"), "{}", error.message);
    assert!(!error.message.contains("synth"), "{}", error.message);
}
