mod support;
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_setup::backends::Config;
use kassiber_store::rusqlite::types::ValueRef;
use support::*;
fn canonical(v: &Value) -> String {
    dumps(
        v,
        &Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .unwrap()
}
fn state(f: &Fixture) -> Value {
    dict(
        [
            ("backends", "SELECT * FROM backends ORDER BY name"),
            (
                "settings",
                "SELECT * FROM settings WHERE key LIKE '%backend%' ORDER BY key",
            ),
        ]
        .map(|(table, sql)| {
            let values = f
                .book
                .connection()
                .prepare(sql)
                .unwrap()
                .query_map([], |r| {
                    Ok(dict(r.as_ref().column_names().iter().enumerate().map(
                        |(i, k)| {
                            (
                                *k,
                                match r.get_ref(i).unwrap() {
                                    ValueRef::Null => Value::Null,
                                    ValueRef::Integer(n) => Value::Int(n.into()),
                                    ValueRef::Text(t) => s(std::str::from_utf8(t).unwrap()),
                                    _ => panic!(),
                                },
                            )
                        },
                    )))
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            (table, Value::List(values))
        }),
    )
}
fn config(f: &Fixture) -> Config {
    let mut c =
        Config::load_with_env(&f._root.path().join("absent.env"), &Default::default()).unwrap();
    c.merge(f.book.connection()).unwrap();
    c
}
fn run(f: &Fixture, c: &mut Config, op: &str, args: Value) -> Value {
    c.merge(f.book.connection()).unwrap();
    c.execute(f.book.connection(), op, &args, &f.sources)
        .unwrap()
}
fn create(f: &Fixture, c: &mut Config, name: &str, kind: &str, chain: &str) -> Value {
    run(
        f,
        c,
        "backends.create",
        dict([
            ("name", s(name)),
            ("kind", s(kind)),
            ("chain", s(chain)),
            ("url", s("https://example.test/api")),
        ]),
    )
}

/// Python's answers with the Rust-only `bitpanda` backend kind
/// (docs/reference/rust-shell.md#rust-only-commands) where Python lists its
/// kinds: the `backends kinds` catalog and the unsupported-kind hint.
fn with_rust_only_kinds(value: &mut Value) {
    match value {
        Value::List(items) => {
            if let Some(at) = items
                .iter()
                .position(|item| get(item, "kind") == Some(&s("bitcoinrpc")))
            {
                items.insert(at + 1, dict([("kind", s("bitpanda"))]));
            }
            items.iter_mut().for_each(with_rust_only_kinds);
        }
        Value::Dict(items) => items
            .iter_mut()
            .for_each(|(_, item)| with_rust_only_kinds(item)),
        Value::Str(text) => {
            if let Some(plain) = text.as_str() {
                if plain.contains("binance, bitcoinrpc, btcpay") {
                    *text = plain
                        .replace(
                            "binance, bitcoinrpc, btcpay",
                            "binance, bitcoinrpc, bitpanda, btcpay",
                        )
                        .into();
                }
            }
        }
        _ => {}
    }
}

#[test]
fn frozen_python_backend_responses_and_database_states() {
    let mut fixture =
        kassiber_pytext::loads(include_str!("../fixtures/python_backends.json")).unwrap();
    with_rust_only_kinds(&mut fixture);
    let Value::List(scenarios) = get(&fixture, "scenarios").unwrap() else {
        panic!()
    };
    for scenario in scenarios {
        let f = Fixture::new();
        if get(scenario, "manual") == Some(&Value::Bool(true)) {
            f.book.connection().execute_batch("INSERT INTO settings(key,value) VALUES('backend_bootstrap_mode','manual'),('bootstrap_backend_tombstones','[\"fulcrum\", \"liquid\", \"liquid-blockstream\", \"mempool\"]')").unwrap();
        }
        let mut c = config(&f);
        let Value::List(records) = get(scenario, "records").unwrap() else {
            panic!()
        };
        for (i, record) in records.iter().enumerate() {
            let op = text(record, "command");
            c.merge(f.book.connection()).unwrap();
            // Refused missing-runtime desktop updates include the env file path.
            // Both sides receive the identical synthetic reference path.
            let result = c.execute(
                f.book.connection(),
                &op,
                get(record, "args").unwrap(),
                &f.sources,
            );
            let outcome = match result {
                Ok(v) => dict([("ok", v)]),
                Err(e) => dict([(
                    "error",
                    dict([
                        ("code", s(e.code)),
                        ("message", s(e.message)),
                        ("hint", e.hint.map_or(Value::Null, s)),
                        ("details", e.details.unwrap_or(Value::Null)),
                        ("retryable", Value::Bool(e.retryable)),
                    ]),
                )]),
            };
            let expected = if let Some(v) = get(record, "ok") {
                dict([("ok", v.clone())])
            } else {
                dict([("error", get(record, "error").unwrap().clone())])
            };
            assert_eq!(
                canonical(&outcome),
                canonical(&expected),
                "record {i}: {op}, manual={:?}",
                get(scenario, "manual")
            );
            assert_eq!(
                canonical(&state(&f)),
                canonical(get(record, "state").unwrap()),
                "state record {i}: {op}"
            );
        }
    }
}

#[test]
fn backend_partial_dotenv_override_preserves_builtin_numeric_defaults() {
    let f = Fixture::new();
    let file = f._root.path().join("backends.env");
    std::fs::write(
        &file,
        "KASSIBER_BACKEND_FULCRUM_URL=ssl://example.test:50002\n",
    )
    .unwrap();
    let mut c = Config::load_with_env(&file, &Default::default()).unwrap();
    let result = run(&f, &mut c, "backends.get", dict([("name", s("fulcrum"))]));
    assert_eq!(get(&result, "batch_size"), Some(&Value::Int(100.into())));
    std::fs::write(&file, "KASSIBER_BACKEND_FULCRUM_BATCH_SIZE=100\n").unwrap();
    let mut c = Config::load_with_env(&file, &Default::default()).unwrap();
    let result = run(&f, &mut c, "backends.get", dict([("name", s("fulcrum"))]));
    assert_eq!(get(&result, "batch_size"), Some(&s("100")));
}

#[test]
fn malformed_backend_locations_never_echo_embedded_credentials() {
    let f = Fixture::new();
    let mut c = config(&f);
    for (i, (field, location)) in [
        (
            "url",
            "https://alice:synthetic-password@[invalid]/api?token=synthetic-token",
        ),
        (
            "url",
            "https://alice:synthetic-password@/api?token=synthetic-token",
        ),
        (
            "url",
            "https://alice:synthetic-password@example.test:synthetic-token/api",
        ),
        (
            "url",
            "https:alice:synthetic-password@example.test?token=synthetic-token",
        ),
        (
            "tor_proxy",
            "socks5h://alice:synthetic-password@[invalid]:9050",
        ),
        ("tor_proxy", "socks5h://alice:synthetic-password@:9050"),
        (
            "tor_proxy",
            "alice:synthetic-password@example.test:synthetic-token",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut args = dict([
            ("name", s(format!("malformed-{i}"))),
            ("kind", s("electrum")),
            ("url", s("ssl://example.test:50002")),
        ]);
        if let Value::Dict(fields) = &mut args {
            fields.retain(|(key, _)| *key != field.into());
            fields.push((field.into(), s(location)));
        }
        let result = run(&f, &mut c, "backends.create", args);
        assert_eq!(get(&result, field), Some(&s("<redacted>")), "{field}");
        let encoded = canonical(&result);
        assert!(!encoded.contains("synthetic-password"));
        assert!(!encoded.contains("synthetic-token"));
    }
}

#[test]
fn backend_default_replacement_stays_on_chain_and_prefers_user_sync_backends() {
    let f = Fixture::new();
    f.book.connection().execute_batch("INSERT INTO settings(key,value) VALUES('backend_bootstrap_mode','manual'),('bootstrap_backend_tombstones','[\"fulcrum\", \"liquid\", \"liquid-blockstream\", \"mempool\"]')").unwrap();
    let mut c = config(&f);
    for (name, kind, chain) in [
        ("aa-commerce", "btcpay", "bitcoin"),
        ("ab-liquid", "electrum", "liquid"),
        ("zz-bitcoin", "electrum", "bitcoin"),
        ("chosen", "esplora", "bitcoin"),
    ] {
        create(&f, &mut c, name, kind, chain);
    }
    run(
        &f,
        &mut c,
        "backends.set-default",
        dict([("name", s("chosen"))]),
    );
    let deleted = run(&f, &mut c, "backends.delete", dict([("name", s("chosen"))]));
    assert_eq!(get(&deleted, "default_backend"), Some(&s("zz-bitcoin")));
    c.merge(f.book.connection()).unwrap();
    let err = c
        .execute(
            f.book.connection(),
            "backends.delete",
            &dict([("name", s("zz-bitcoin"))]),
            &f.sources,
        )
        .unwrap_err();
    assert_eq!(err.code, "conflict");
    assert!(err.message.contains("No bitcoin backend"));
    assert!(f
        .book
        .connection()
        .query_row("SELECT 1 FROM backends WHERE name='zz-bitcoin'", [], |r| {
            r.get::<_, i64>(0)
        })
        .is_ok());
}

#[test]
fn backend_wallet_and_btcpay_provenance_references_block_deletion() {
    let f = Fixture::seed();
    let mut c = config(&f);
    create(&f, &mut c, "private", "esplora", "bitcoin");
    let p = f.profile();
    let pid = text(&p, "id");
    let wid = text(&p, "workspace_id");
    f.book.connection().execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet',?,?,'Settlement','address_list','{\"btcpay_provenance\":[{\"backend\":\" PRIVATE \"}]}','t')",[wid,pid]).unwrap();
    let err = c
        .execute(
            f.book.connection(),
            "backends.delete",
            &dict([("name", s("private"))]),
            &f.sources,
        )
        .unwrap_err();
    assert_eq!(err.code, "conflict");
    assert_eq!(
        get(err.details.as_ref().unwrap(), "wallet_refs"),
        Some(&Value::List(vec![s("Main/Book/Settlement")]))
    );
}

#[test]
fn backend_secrets_are_stored_exactly_and_allowlisted_output_never_discloses_them() {
    let f = Fixture::new();
    let mut c = config(&f);
    let args=kassiber_pytext::loads(r#"{"name":"private","kind":"bitcoinrpc","url":"https://alice:synthetic-secret@EXAMPLE.test:8443/api?key=synthetic-query#synthetic-fragment","token":" synthetic-token ","auth_header":"Bearer synthetic-header","tor_proxy":"bob:synthetic-proxy@localhost:9050","config":{"rpc_password":"  synthetic-password  ","macaroon":"synthetic-macaroon","unknown":"synthetic-unknown","certificate":"/private/ca.pem","rpc_user":"synthetic-user"}}"#).unwrap();
    let result = run(&f, &mut c, "backends.create", args);
    let raw = f
        .book
        .connection()
        .query_row(
            "SELECT token,auth_header,config_json FROM backends WHERE name='private'",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(raw.0, " synthetic-token ");
    assert_eq!(raw.1, "Bearer synthetic-header");
    assert!(raw.2.contains("synthetic-macaroon"));
    assert!(raw.2.contains("\"password\": \"synthetic-password\""));
    for out in [
        result,
        run(&f, &mut c, "backends.get", dict([("name", s("private"))])),
        run(&f, &mut c, "backends.list", dict::<&str>([])),
        run(&f, &mut c, "ui.backends.options", dict::<&str>([])),
    ] {
        let rendered = canonical(&out);
        assert!(!rendered.contains("synthetic-"), "{rendered}");
        assert!(!rendered.contains("/private/ca.pem"));
    }
    let settings = canonical(&run(
        &f,
        &mut c,
        "ui.backends.settings.list",
        dict::<&str>([]),
    ));
    assert!(settings.contains("/private/ca.pem"));
    assert!(!settings.contains("synthetic-"));
    let revealed = run(
        &f,
        &mut c,
        "backends.reveal-token",
        dict([("name", s("private"))]),
    );
    assert_eq!(get(&revealed, "token"), Some(&s(" synthetic-token ")));
    assert_eq!(get(&revealed, "password"), Some(&s("synthetic-password")));
}

#[test]
fn backend_environment_overrides_are_ephemeral_and_cannot_be_deleted_or_defaulted() {
    let f = Fixture::new();
    let env = [
        ("KASSIBER_BACKEND_EPHEMERAL_KIND", "electrum"),
        ("KASSIBER_BACKEND_EPHEMERAL_URL", "ssl://example.test:50002"),
        ("KASSIBER_BACKEND_EPHEMERAL_TOKEN", "synthetic-env-token"),
    ]
    .map(|(k, v)| (k.into(), v.into()))
    .into_iter()
    .collect();
    let mut c = Config::load_with_env(&f._root.path().join("absent.env"), &env).unwrap();
    c.seed_configuration(f.book.connection()).unwrap();
    c.merge(f.book.connection()).unwrap();
    assert_eq!(
        f.book
            .connection()
            .query_row(
                "SELECT count(*) FROM backends WHERE name='ephemeral'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    for op in ["backends.set-default", "backends.delete"] {
        let err = c
            .execute(
                f.book.connection(),
                op,
                &dict([("name", s("ephemeral"))]),
                &f.sources,
            )
            .unwrap_err();
        assert_eq!(err.code, "conflict");
    }
    create(&f, &mut c, "ephemeral", "electrum", "bitcoin");
    let out = run(&f, &mut c, "backends.get", dict([("name", s("ephemeral"))]));
    assert_eq!(get(&out, "source"), Some(&s("environment")));
    assert_eq!(get(&out, "has_token"), Some(&Value::Bool(true)));
    assert_eq!(
        f.book
            .connection()
            .query_row(
                "SELECT token FROM backends WHERE name='ephemeral'",
                [],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
}

#[test]
fn backend_deleted_presets_stay_deleted_after_restart_and_seed() {
    let f = Fixture::new();
    let mut c = config(&f);
    c.seed_configuration(f.book.connection()).unwrap();
    c.merge(f.book.connection()).unwrap();
    run(
        &f,
        &mut c,
        "backends.delete",
        dict([("name", s("mempool"))]),
    );
    let mut reopened = config(&f);
    reopened.seed_configuration(f.book.connection()).unwrap();
    reopened.merge(f.book.connection()).unwrap();
    let listed = canonical(&run(&f, &mut reopened, "backends.list", dict::<&str>([])));
    assert!(!listed.contains("\"name\": \"mempool\""));
    assert_eq!(
        f.book
            .connection()
            .query_row(
                "SELECT count(*) FROM backends WHERE name='mempool'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    create(&f, &mut reopened, "mempool", "esplora", "bitcoin");
    assert_eq!(
        get(
            &run(
                &f,
                &mut reopened,
                "backends.get",
                dict([("name", s("mempool"))])
            ),
            "source"
        ),
        Some(&s("database"))
    );
}

#[test]
fn backend_update_clears_only_its_cached_inventory() {
    let f = Fixture::seed();
    let mut c = config(&f);
    create(&f, &mut c, "private", "esplora", "bitcoin");
    for backend in ["private", "other"] {
        let p = f.profile();
        let wid = text(&p, "workspace_id");
        let pid = text(&p, "id");
        f.book.connection().execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES(?,?,?,?,'address','t')",[backend,&wid,&pid,backend]).unwrap();
        f.book.connection().execute("INSERT INTO wallet_utxo_refreshes(wallet_id,workspace_id,profile_id,backend_name,chain,network,last_seen_at) VALUES(?,?,?,?,'bitcoin','main','t')",[backend,&wid,&pid,backend]).unwrap();
    }
    run(
        &f,
        &mut c,
        "backends.update",
        dict([("name", s("private")), ("notes", s("Edited"))]),
    );
    assert_eq!(
        f.book
            .connection()
            .query_row(
                "SELECT count(*) FROM wallet_utxo_refreshes WHERE backend_name='private'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        f.book
            .connection()
            .query_row(
                "SELECT count(*) FROM wallet_utxo_refreshes WHERE backend_name='other'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}
