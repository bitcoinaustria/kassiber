use super::*;

fn fixture() -> Connection {
    fixture_codec(false)
}
fn fixture_codec(encrypted: bool) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    if encrypted {
        conn.execute_batch("PRAGMA key='disposable-test-key'")
            .unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let creation: Json =
        serde_json::from_str(include_str!("../../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01'),('q','ws','Other','2025-01-01');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet','ws','p','Source','custom','{}','2025-01-01');").unwrap();
    conn
}
fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn import(conn: &Connection, content: &str) -> Result<Py, AppError> {
    import_format(conn, content, false)
}
fn import_format(conn: &Connection, content: &str, phoenix: bool) -> Result<Py, AppError> {
    import_kind(
        conn,
        content,
        if phoenix {
            Format::Phoenix
        } else {
            Format::Csv
        },
    )
}
fn import_kind(conn: &Connection, content: &str, format: Format) -> Result<Py, AppError> {
    let file = std::env::temp_dir().join(format!(
        "kassiber-import-{}-{:?}.csv",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&file, content).unwrap();
    let result = execute(
        conn,
        Request {
            profile_id: "p".into(),
            wallet: "Source".into(),
            file: file.to_str().unwrap().into(),
            format,
            mode: None,
        },
        None,
    );
    std::fs::remove_file(file).unwrap();
    result
}

#[test]
fn phoenix_metadata_is_idempotent_preserves_authored_notes_and_invalidates_once() {
    let conn = fixture();
    let content="date,id,type,amount_msat,mining_fee_sat,service_fee_msat,description,payment_hash\n2025-01-01,ln,lightning_received,3001,2,7, imported note ,ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB\n";
    let result = import_format(&conn, content, true).unwrap();
    assert_eq!(scalar(&conn, "SELECT amount FROM transactions"), 3001);
    assert_eq!(scalar(&conn, "SELECT fee FROM transactions"), 2007);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM tags WHERE code='lightning_received'"
        ),
        1
    );
    assert_eq!(
        kassiber_envelope::get(&result, "phoenix_notes_set"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "phoenix_tags_added"),
        Some(&Py::from(1))
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
    let stored: (String, String, String) = conn
        .query_row(
            "SELECT note,payment_hash,payment_hash_source FROM transactions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        ("imported note".into(), "ab".repeat(32), "importer".into())
    );
    let result = import_format(&conn, content, true).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "journal_invalidated"),
        Some(&Py::Bool(false))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "phoenix_notes_set"),
        Some(&Py::from(0))
    );
    conn.execute_batch("DELETE FROM transaction_tags;UPDATE transactions SET note=NULL;")
        .unwrap();
    let result = import_format(&conn, content, true).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "unchanged"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "journal_invalidated"),
        Some(&Py::Bool(true))
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        2
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    conn.execute("UPDATE transactions SET note='authored'", [])
        .unwrap();
    import_format(&conn, content, true).unwrap();
    let stored: String = conn
        .query_row("SELECT note FROM transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, "authored");
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 1);
}
#[test]
fn csv_exchange_enrichment_respects_metadata_authorship_and_price_rank() {
    let conn = fixture();
    let header = "date,txid,amount,fiat_rate,pricing_source_kind,kind,description,counterparty\n";
    import(
        &conn,
        &format!("{header}2025-01-01,fill,1,10,generic_import,deposit,quote,broker\n"),
    )
    .unwrap();
    import(
        &conn,
        &format!("{header}2025-01-01,fill,1,11,exchange_execution,buy,fill,exchange\n"),
    )
    .unwrap();
    let metadata = || {
        conn.query_row(
            "SELECT kind,description,counterparty FROM transactions",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap()
    };
    assert_eq!(metadata(), ("buy".into(), "fill".into(), "exchange".into()));
    conn.execute_batch("UPDATE transactions SET kind='authored kind',description='authored description',counterparty='authored party';").unwrap();
    import(
        &conn,
        &format!("{header}2025-01-01,fill,1,12,exchange_execution,buy,new fill,new exchange\n"),
    )
    .unwrap();
    assert_eq!(
        metadata(),
        (
            "authored kind".into(),
            "authored description".into(),
            "authored party".into()
        )
    );
    // A higher-priority saved price also protects import-authored metadata.
    conn.execute_batch("UPDATE transactions SET kind='buy',description='new fill',counterparty='new exchange',pricing_source_kind='manual_override';").unwrap();
    import(
        &conn,
        &format!("{header}2025-01-01,fill,1,13,exchange_execution,sell,lower quote,lower party\n"),
    )
    .unwrap();
    assert_eq!(
        metadata(),
        ("buy".into(), "new fill".into(), "new exchange".into())
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
}

#[test]
fn csv_evidence_enrichment_preserves_authored_boundaries_and_promotes_exact_links() {
    let conn = fixture();
    let content = |boundary: &str, source: &str| {
        format!("date,txid,amount,privacy_boundary,raw_json\n2025-01-01,evidence,1,{boundary},\"{{\"\"source\"\":\"\"{source}\"\",\"\"privacy_boundary\"\":\"\"{boundary}\"\"}}\"\n")
    };
    import(&conn, &content("payjoin", "wallet")).unwrap();
    import(&conn, &content("sweep", "wallet")).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE privacy_boundary='sweep'"
        ),
        1
    );
    import(&conn, &content("", "other")).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE privacy_boundary='sweep'"
        ),
        1
    );
    import(&conn, &content("", "wallet")).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE privacy_boundary IS NULL"
        ),
        1
    );
    import(&conn, &content("payjoin", "wallet")).unwrap();
    conn.execute_batch("UPDATE transactions SET privacy_boundary='coinjoin';")
        .unwrap();
    import(&conn, &content("sweep", "wallet")).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE privacy_boundary='coinjoin'"
        ),
        1
    );
    let content = |source: &str, vout: &str| {
        format!("date,txid,amount,payment_hash,payment_hash_source,swap_refund_funding_txid,swap_refund_funding_vout\n2025-01-01,evidence,1,{},{source},{},{vout}\n","ab".repeat(32),"cd".repeat(32))
    };
    import(&conn, &content("chain_script", "")).unwrap();
    import(&conn, &content("chain_script_unique_outpoint", "0")).unwrap();
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE payment_hash_source='chain_script_unique_outpoint' AND swap_refund_funding_vout=0"),1);
    import(&conn, &content("chain_script", "1")).unwrap();
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE payment_hash_source='chain_script_unique_outpoint' AND swap_refund_funding_vout=0"),1);
    let content = |kind: &str, source: &str| {
        format!("date,txid,amount,kind,raw_json\n2025-01-01,wasabi,1,{kind},\"{{\"\"source\"\":\"\"{source}\"\"}}\"\n")
    };
    import(&conn, &content("deposit", "wasabi_gethistory")).unwrap();
    import(&conn, &content("withdrawal", "other")).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE external_id='wasabi' AND kind='deposit'"
        ),
        1
    );
    import(&conn, &content("coinjoin", "wasabi_gethistory")).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE external_id='wasabi' AND kind='coinjoin'"
        ),
        1
    );
}

#[test]
fn csv_import_dedupe_batch_membership_and_journal_invalidation() {
    let conn = fixture();
    let content="date,txid,direction,asset,amount,fee,description\n2025-01-02,same,in,BTC,0.000000000015,0.000000000005,\n2025-01-03,same,in,BTC,0.000000000015,0.000000000005,added\n2025-01-02,,in,BTC,1,0,anonymous\n2025-01-02,,in,BTC,1,0,anonymous\n";
    let result = import(&conn, content).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(2))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "skipped"),
        Some(&Py::from(2))
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT amount FROM transactions WHERE external_id='same'"
        ),
        2
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT fee FROM transactions WHERE external_id='same'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE external_id IS NULL"
        ),
        1
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM import_batch_transactions"),
        2
    );
    assert_eq!(scalar(&conn, "SELECT rows_inserted FROM import_batches"), 2);
    assert_eq!(scalar(&conn, "SELECT rows_updated FROM import_batches"), 0);
    assert_eq!(scalar(&conn, "SELECT rows_skipped FROM import_batches"), 2);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='q'"
        ),
        0
    );
    let result = import(&conn, content).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "journal_invalidated"),
        Some(&Py::Bool(false))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
}
#[test]
fn csv_import_failures_are_atomic_and_empty_runs_preserve_readiness() {
    let conn = fixture();
    import(&conn, "date,amount\n").unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        0
    );
    assert!(import(&conn, "date,amount\n2025-01-01,1\ninvalid,2\n").is_err());
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
    conn.execute_batch("CREATE TRIGGER fail_batch BEFORE INSERT ON import_batches BEGIN SELECT RAISE(ABORT,'batch denied');END;").unwrap();
    assert!(import(&conn, "date,amount\n2025-01-01,1\n").is_err());
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        0
    );
    assert!(conn.is_autocommit());
}

#[test]
fn csv_enrichment_preserves_closed_observer_evidence() {
    let conn = fixture();
    let id = "a".repeat(64);
    import(&conn, &format!("date,txid,amount\n2025-01-01,{id},1\n")).unwrap();
    let raw = r#"{"observer":"synthetic"}"#;
    conn.execute("UPDATE transactions SET raw_json=?", [raw])
        .unwrap();
    let record = normalize(&csv::Row {
        headers: vec![],
        original: None,
        data: json!({"date":"2025-01-01","txid":id,"amount":"1"}),
        rest: None,
    })
    .unwrap();
    let row = find(&conn, "wallet", &record, &fingerprint("wallet", &record))
        .unwrap()
        .unwrap();
    let value = json!({"wallet_id":"wallet","external_id":id,"direction":"inbound","asset":"BTC","amount":100000000000_i64,"fee":0,"amount_includes_fee":0});
    let graph = kassiber_custody::evidence::graph_hash(&json!(raw))
        .unwrap()
        .0;
    let quantity = kassiber_custody::evidence::observed_hash(&value).unwrap().0;
    conn.execute("INSERT INTO chain_observation_provenance(transaction_id,workspace_id,profile_id,wallet_id,authority_version,observer_ids_json,observer_kinds_json,chain,network,application_revision,graph_hash,quantity_hash,fee_attribution,observed_at,updated_at) VALUES(?,'ws','p','wallet',1,'[]','[]','bitcoin','main','r',?,?,'exact','2025-01-01','2025-01-01')",params![string(&row,"id"),graph,quantity]).unwrap();
    import(
        &conn,
        &format!("date,txid,amount,description\n2025-01-01,{id},1,added\n"),
    )
    .unwrap();
    let stored: String = conn
        .query_row("SELECT raw_json FROM transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, raw);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
}

#[test]
fn csv_dialect_preserves_python_rows() {
    assert_eq!(
        parsed_rows("a,b,c\r\n\r\n1,\"x,\"\"y\r\nz\",3\r4,\"unclosed\n").unwrap(),
        vec![
            json!({"a":"1","b":"x,\"y\r\nz","c":"3"}),
            json!({"a":"4","b":"unclosed\n","c":null})
        ]
    );
    assert_eq!(
        parsed_rows("\u{feff}date,a,a\nx,1,2\n").unwrap(),
        vec![json!({"\u{feff}date":"x","a":"2"})]
    );
    assert_eq!(
        parsed_rows("a,b\n1,\"x\"tail\n").unwrap(),
        vec![json!({"a":"1","b":"xtail"})]
    );
}

#[test]
fn python_file_import_vectors() {
    let cases: Json =
        serde_json::from_str(include_str!("../../tests/vectors/python_imports.json")).unwrap();
    assert_import_vectors(&cases);
}
#[test]
fn python_river_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_exchange_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}
fn assert_import_vectors(cases: &Json) {
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes = case.get("bytes").map(|v| {
            v.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        });
        let path = std::env::temp_dir().join(format!(
            "kassiber-import-vector-{}-{:?}-{}{}",
            std::process::id(),
            std::thread::current().id(),
            name.bytes().map(u32::from).sum::<u32>(),
            case["extension"].as_str().unwrap_or(".csv")
        ));
        std::fs::write(
            &path,
            bytes
                .as_deref()
                .unwrap_or_else(|| case["text"].as_str().unwrap().as_bytes()),
        )
        .unwrap();
        let format = match case["format"].as_str().unwrap() {
            "json" => Format::Json,
            "btcpay_csv" => Format::BtcpayCsv,
            "btcpay_json" => Format::BtcpayJson,
            "generic_ledger" => Format::Ledger,
            "phoenix_csv" => Format::Phoenix,
            "river_csv" => Format::River,
            "bullbitcoin_csv" => Format::Bull,
            "coinfinity_csv" => Format::Coinfinity,
            "21bitcoin_csv" => Format::TwentyOne,
            "strike_csv" => Format::Strike,
            "ledgerlive_csv" => Format::LedgerLive,
            "cointracking_csv" => Format::CoinTracking,
            "blockpit_csv" => Format::Blockpit,
            "binance_supplemental_csv" => Format::Binance,
            "wasabi_bundle" => Format::Wasabi,
            "pocketbitcoin_csv" => Format::Pocket,
            _ => Format::Csv,
        };
        let mapping = case.get("column_map").filter(|v| !v.is_null()).map(|v| {
            ledger::ColumnMap::from_python(
                &case
                    .get("column_map_json")
                    .and_then(Json::as_str)
                    .map(|s| kassiber_pytext::loads(s).unwrap())
                    .unwrap_or_else(|| kassiber_pytext::loads(&v.to_string()).unwrap()),
            )
            .unwrap()
        });
        let loaded = if format == Format::Ledger {
            ledger::load(path.to_str().unwrap(), mapping.as_ref())
        } else {
            format.load(path.to_str().unwrap())
        };
        let result = loaded.and_then(|rows| {
            rows.iter()
                .map(|row| {
                    let r = normalize(row)?;
                    if let Some(error) = r.insert_binding_error() { return Err(error.clone()); }
                    let fields: serde_json::Map<String, Json> = r
                        .fields
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                match v {
                                    Sql::Null => Json::Null,
                                    Sql::Integer(n) => json!(n),
                                    Sql::Real(n) if n.is_infinite() => json!({"$float":if n.is_sign_negative() {"-inf"} else {"inf"}}),
                                    Sql::Real(n) => json!(n),
                                    Sql::Text(v) => json!(v),
                                    _ => unreachable!(),
                                },
                            )
                        })
                        .collect();
                    Ok(json!({"fields":fields,"fingerprint":fingerprint("wallet",&r)}))
                })
                .collect::<Result<Vec<_>, AppError>>()
        });
        std::fs::remove_file(&path).unwrap();
        match result {
            Ok(records) => assert_eq!(
                kassiber_pytext::loads(&json!(records).to_string()).unwrap(),
                kassiber_pytext::loads(&case["records"].to_string()).unwrap(),
                "{name}"
            ),
            Err(error) => {
                if let Some(expected) = case["error"].get("details_json") {
                    let details = error
                        .details
                        .as_ref()
                        .map(|v| kassiber_pyjson::dumps(v, &Default::default()).unwrap())
                        .unwrap_or_else(|| "null".into());
                    assert_eq!(json!(details), *expected, "{name}");
                }
                assert_eq!(
                    json!({"code":error.code,"message":error.message.replace(path.to_str().unwrap(),"{file}"),"hint":error.hint}),
                    json!({"code":case["error"]["code"],"message":case["error"]["message"],"hint":case["error"]["hint"]}),
                    "{name}"
                );
            }
        }
    }
}

fn parsed_rows(text: &str) -> Result<Vec<Json>, AppError> {
    csv::rows(text).map(|rows| rows.into_iter().map(|r| r.data).collect())
}

#[test]
fn json_import_identity_fee_relaxation_routing_and_atomicity() {
    let conn = fixture();
    let run = |value: Json| import_kind(&conn, &value.to_string(), Format::Json);
    let idless = json!([
        {"date":"2025-01-01","amount":1},
        {"date":"2025-01-02","amount":1},
        {"date":"2025-01-02","amount":1}
    ]);
    let result = run(idless.clone()).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(2))
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM import_batch_transactions"),
        2
    );
    let result = run(idless).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "unchanged"),
        Some(&Py::from(3))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    conn.execute(
        "UPDATE transactions SET external_id='' WHERE occurred_at='2025-01-01T00:00:00Z'",
        [],
    )
    .unwrap();
    let result = run(json!([{"date":"2025-01-03","amount":1}])).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(1))
    );
    run(json!([{"date":"2025-01-03","txid":"fee","amount":1,"fee":0}])).unwrap();
    let result = run(json!([{"date":"2025-01-03","txid":"fee","amount":1,"fee":0.01,"fiat_value":10,"pricing_source_kind":"exchange_execution"}])).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT fee FROM transactions WHERE external_id='fee'"
        ),
        0
    );
    run(json!([{"date":"2025-01-03","txid":"ignore","amount":1}])).unwrap();
    let result = run(json!([{"date":"2025-01-03","txid":"ignore","amount":1,"fee":0.01,"_match_existing_ignore_fee":true}])).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(0))
    );
    run(json!([{"date":"2025-01-03","txid":"routing","amount":1,"kind":"routing_income","fiat_rate":10}])).unwrap();
    run(json!([{"date":"2025-01-03","txid":"routing","amount":2,"fee":0.00000000001,"kind":"routing_income"}])).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT amount FROM transactions WHERE external_id='routing'"
        ),
        200000000000
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT fee FROM transactions WHERE external_id='routing'"
        ),
        1
    );
    let value: String = conn
        .query_row(
            "SELECT fiat_value_exact FROM transactions WHERE external_id='routing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(value, "20");
    run(json!([{"date":"2025-01-03","txid":"unpriced","amount":1,"kind":"routing_income","fiat_value":10}])).unwrap();
    // Current imports derive a rate from the supplied value. Keep this
    // recovery witness about legacy rows whose stored rate is absent.
    let rate: String = conn
        .query_row(
            "SELECT fiat_rate_exact FROM transactions WHERE external_id='unpriced'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rate, "10");
    conn.execute(
        "UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL WHERE external_id='unpriced'",
        [],
    )
    .unwrap();
    run(json!([{"date":"2025-01-03","txid":"unpriced","amount":2,"kind":"routing_income"}]))
        .unwrap();
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE external_id='unpriced' AND fiat_value IS NULL AND fiat_value_exact IS NULL"),1);
    run(json!([{"date":"2025-01-04","amount":1,"_tax_platform_row_identity":"row-identity"}]))
        .unwrap();
    run(json!([{"date":"2025-01-04","amount":1,"_tax_platform_row_identity":"row-identity","txid":"ab".repeat(32)}])).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE occurred_at='2025-01-04T00:00:00Z'"
        ),
        1
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE occurred_at='2025-01-04T00:00:00Z' AND external_id_kind='txid'"),1);
    let version = scalar(
        &conn,
        "SELECT journal_input_version FROM profiles WHERE id='p'",
    );
    let count = scalar(&conn, "SELECT count(*) FROM transactions");
    let error =
        run(json!([{"date":"2025-01-05","amount":1},{"date":"invalid","amount":1}])).unwrap_err();
    assert_eq!(error.code, "validation");
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), count);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        version
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='q'"
        ),
        0
    );
}

#[test]
fn json_metadata_binding_is_deferred_until_a_field_is_written() {
    let conn = fixture();
    let run = |value: Json| import_kind(&conn, &value.to_string(), Format::Json);
    run(json!([{"date":"2025-01-01","txid":"authored","amount":1,"description":"authored"}]))
        .unwrap();
    run(json!([{"date":"2025-01-01","txid":"authored","amount":1,"description":[1]}])).unwrap();
    run(json!([{"date":"2025-01-01","txid":"authored","amount":1,"kind":[]}])).unwrap();
    import_kind(
        &conn,
        r#"[{"date":"2025-01-01","txid":"authored","amount":1,"raw_json":"\ud800"}]"#,
        Format::Json,
    )
    .unwrap();
    let version = scalar(
        &conn,
        "SELECT journal_input_version FROM profiles WHERE id='p'",
    );
    let error =
        run(json!([{"date":"2025-01-01","txid":"authored","amount":1,"kind":[1]}])).unwrap_err();
    assert_eq!(error.code, "internal_error");
    assert_eq!(
        error.message,
        "Error binding parameter 1: type 'list' is not supported"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        version
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    let error =
        run(json!([{"date":"2025-01-02","txid":"new","amount":1,"description":[]}])).unwrap_err();
    assert_eq!(
        error.message,
        "Error binding parameter 32: type 'list' is not supported"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    let error = run(json!([{"date":"2025-01-02","txid":"new","amount":1,"kind":[],"description":{},"counterparty":[1]}])).unwrap_err();
    assert_eq!(
        error.message,
        "Error binding parameter 30: type 'list' is not supported"
    );
}

#[test]
fn json_sqlcipher_binding_refusal_preserves_the_reference_driver_message() {
    let conn = fixture_codec(true);
    let error = import_kind(
        &conn,
        r#"[{"date":"2025-01-01","amount":1,"kind":[]}]"#,
        Format::Json,
    )
    .unwrap_err();
    assert_eq!(
        error.message,
        "Error binding parameter 29 - probably unsupported type."
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        0
    );
}

#[test]
fn river_metadata_preserves_authored_notes_and_invalidates_once() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    let content = "Date,Sent Amount,Sent Currency,Received Amount,Received Currency,Fee Amount,Fee Currency,Reference Code,Transaction Type,Tag,Method\n2025-01-02,,,0.002,BTC,,,buy,Buy,Buy Reward,wire\n";
    let result = import_kind(&conn, content, Format::River).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "river_tags_created"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "river_notes_set"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "river_tags_added"),
        Some(&Py::from(1))
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM tags WHERE code='river:buy_reward' AND label='Buy Reward'"
        ),
        1
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE amount=200000000 AND fiat_value_exact IS NULL AND fiat_rate_exact IS NULL AND note='Buy - wire'"),1);
    let repeated = import_kind(&conn, content, Format::River).unwrap();
    assert_eq!(
        kassiber_envelope::get(&repeated, "journal_invalidated"),
        Some(&Py::Bool(false)),
        "{repeated:?}"
    );
    assert_eq!(
        kassiber_envelope::get(&repeated, "river_tags_created"),
        Some(&Py::from(0))
    );
    conn.execute_batch("DELETE FROM transaction_tags; UPDATE transactions SET note=NULL")
        .unwrap();
    let enriched = import_kind(&conn, content, Format::River).unwrap();
    assert_eq!(
        kassiber_envelope::get(&enriched, "journal_invalidated"),
        Some(&Py::Bool(true))
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        2
    );
    conn.execute_batch("UPDATE transactions SET note='authored'")
        .unwrap();
    import_kind(&conn, content, Format::River).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE note='authored'"
        ),
        1
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 1);
}
#[test]
fn river_tag_codes_lowercase_the_provider_prefix_and_tag_together() {
    let conn = fixture();
    import_kind(&conn, "Date,Sent Amount,Sent Currency,Received Amount,Received Currency,Reference Code,Tag\n2025-01-02,,,0.002,BTC,greek,Σ\n", Format::River).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM tags WHERE code='river:ς' AND label='Σ'"
        ),
        1
    );
}

#[test]
fn river_metadata_failure_rolls_back_observations_tags_and_run() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    conn.execute_batch("CREATE TRIGGER refuse_river_tag BEFORE INSERT ON transaction_tags BEGIN SELECT RAISE(ABORT, 'river metadata failure'); END").unwrap();
    let error = import_kind(&conn,"Date,Sent Amount,Sent Currency,Received Amount,Received Currency,Reference Code,Tag\n2025-01-02,100,EUR,0.002,BTC,buy,Buy\n",Format::River).unwrap_err();
    assert_eq!(error.code, "internal_error", "{error:?}");
    for table in [
        "transactions",
        "tags",
        "transaction_tags",
        "import_batches",
        "import_batch_transactions",
    ] {
        assert_eq!(
            scalar(&conn, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        0
    );
}

#[test]
fn python_bull_import_vectors() {
    let cases: Json =
        serde_json::from_str(include_str!("../../tests/vectors/python_bull_imports.json")).unwrap();
    assert_import_vectors(&cases);
}
#[test]
fn python_coinfinity_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_coinfinity_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}
#[test]
fn python_twentyone_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_twentyone_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

fn bull(conn: &Connection, mode: &str) -> Py {
    let rows = bull::parse(csv::rows("ORDER_STATUS,PAYIN_AMOUNT,PAYIN_CURRENCY,PAYOUT_AMOUNT,PAYOUT_CURRENCY,COMPLETED_AT (UTC),TRANSACTION_ID\ncompleted,100,EUR,0.002,BTC,2025-01-02,match\n").unwrap()).unwrap();
    let records = rows
        .iter()
        .map(normalize)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let request = Request {
        profile_id: "p".into(),
        wallet: "Source".into(),
        file: "bull.csv".into(),
        format: Format::Bull,
        mode: Some(mode.into()),
    };
    if mode == "relevant" {
        exchange::enrich(conn, &request, &rows).unwrap()
    } else {
        apply(
            conn,
            &request,
            &resolve(conn, "p", "Source").unwrap(),
            &rows,
            &records,
            &mut Ids {
                identity: None,
                at: 0,
            },
            None,
        )
        .unwrap()
    }
}
#[test]
fn bull_profile_matching_fee_precedence_ambiguity_and_scope() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    import(
        &conn,
        "date,txid,amount,fee\n2025-01-01,match,0.002,0.00001\n",
    )
    .unwrap();
    // Fee fallback enriches the observed row without rewriting its fee/date/id.
    let result = bull(&conn, "relevant");
    assert_eq!(
        kassiber_envelope::get(&result, "matched"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn, "SELECT fee FROM transactions"), 1000000);
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE kind='buy' AND fiat_value_exact='100' AND occurred_at='2025-01-01T00:00:00Z'"),1);
    conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('other','ws','p','Other','custom','{}','2025-01-01'),('foreign','ws','q','Foreign','custom','{}','2025-01-01');").unwrap();
    let row=bull::parse(csv::rows("ORDER_STATUS,PAYIN_AMOUNT,PAYIN_CURRENCY,PAYOUT_AMOUNT,PAYOUT_CURRENCY,COMPLETED_AT (UTC),TRANSACTION_ID\ncompleted,100,EUR,0.002,BTC,2025-01-02,match\n").unwrap()).unwrap();
    let r = normalize(&row[0]).unwrap();
    let mut fields = r.fields.clone();
    for (k, v) in [
        ("id", "exact"),
        ("workspace_id", "ws"),
        ("profile_id", "p"),
        ("wallet_id", "other"),
        ("created_at", "2025-01-01"),
        ("fingerprint", "other-fp"),
    ] {
        fields.insert(k.into(), Sql::Text(v.into()));
    }
    insert(&conn, "transactions", &fields).unwrap();
    let found = exchange::find_profile(&conn, "p", &r, None).unwrap();
    assert_eq!(
        string(&found.0.unwrap(), "id"),
        "exact",
        "exact fee wins over amount-only match"
    );
    assert_eq!(
        exchange::find_profile(&conn, "p", &r, Some("other"))
            .unwrap()
            .1,
        "matched"
    );
    conn.execute("UPDATE transactions SET fee=1 WHERE id='exact'", [])
        .unwrap();
    assert_eq!(
        exchange::find_profile(&conn, "p", &r, None).unwrap().1,
        "ambiguous"
    );
    assert_eq!(
        kassiber_envelope::get(&bull(&conn, "relevant"), "skipped_ambiguous"),
        Some(&Py::from(1))
    );
    conn.execute("UPDATE transactions SET fee=0", []).unwrap();
    assert_eq!(
        exchange::find_profile(&conn, "p", &r, None).unwrap().1,
        "ambiguous"
    );
    conn.execute(
        "UPDATE transactions SET profile_id='q',wallet_id='foreign' WHERE id='exact'",
        [],
    )
    .unwrap();
    assert_eq!(
        exchange::find_profile(&conn, "p", &r, None).unwrap().1,
        "matched"
    );
    for (column, value) in [
        ("direction", "outbound"),
        ("asset", "LBTC"),
        ("external_id", "different"),
    ] {
        conn.execute(
            &format!("UPDATE transactions SET {column}=? WHERE wallet_id='wallet'"),
            [value],
        )
        .unwrap();
        assert_eq!(
            exchange::find_profile(&conn, "p", &r, None).unwrap().1,
            "unmatched",
            "{column}"
        );
        conn.execute(
            &format!("UPDATE transactions SET {column}=? WHERE wallet_id='wallet'"),
            [string(&r.fields, column)],
        )
        .unwrap();
    }
    conn.execute(
        "UPDATE transactions SET amount=1 WHERE wallet_id='wallet'",
        [],
    )
    .unwrap();
    assert_eq!(
        exchange::find_profile(&conn, "p", &r, None).unwrap().1,
        "unmatched"
    );
    assert_eq!(
        kassiber_envelope::get(&bull(&conn, "relevant"), "skipped_unmatched"),
        Some(&Py::from(1))
    );
}
#[test]
fn bull_full_exclusion_reconciliation_dedupe_and_readiness() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    let first = bull(&conn, "full");
    assert_eq!(
        kassiber_envelope::get(&first, "unmatched"),
        Some(&Py::from(1))
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transactions WHERE excluded=1"),
        1
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM tags"), 3);
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='bullbitcoin-wallet-gap' AND label='Bull Bitcoin wallet gap'"),1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        2
    );
    let repeat = bull(&conn, "full");
    assert_eq!(
        kassiber_envelope::get(&repeat, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(
        kassiber_envelope::get(&repeat, "updated"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&repeat, "unmatched"),
        Some(&Py::from(1)),
        "exclude destination wallet before insertion"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        4
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='q'"
        ),
        0
    );
    conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('other','ws','p','Other','custom','{}','2025-01-01');
        INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('observed','ws','p','other','match','o','2025-01-01','inbound','BTC',200000000,0,'2025-01-01');").unwrap();
    let matched = bull(&conn, "full");
    assert_eq!(
        kassiber_envelope::get(&matched, "matched"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='bullbitcoin-matched' AND label='Bull Bitcoin matched'"),1);
    conn.execute_batch("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('sibling','ws','p','other','match','s','2025-01-01','inbound','BTC',200000000,0,'2025-01-01');").unwrap();
    let ambiguous = bull(&conn, "full");
    assert_eq!(
        kassiber_envelope::get(&ambiguous, "ambiguous"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='bullbitcoin-ambiguous' AND label='Bull Bitcoin ambiguous'"),1);
}
#[test]
fn bull_coordinator_failure_rolls_back_transactions_flags_run_and_invalidation() {
    for step in ["tags", "transaction_tags", "import_batches"] {
        let conn = fixture();
        conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
            .unwrap();
        conn.execute_batch(&format!(
            "CREATE TRIGGER deny BEFORE INSERT ON {step} BEGIN SELECT RAISE(ABORT,'denied');END;"
        ))
        .unwrap();
        let file =
            std::env::temp_dir().join(format!("bull-atomic-{}-{step}.csv", std::process::id()));
        std::fs::write(&file,"ORDER_STATUS,PAYIN_AMOUNT,PAYIN_CURRENCY,PAYOUT_AMOUNT,PAYOUT_CURRENCY,COMPLETED_AT (UTC),TRANSACTION_ID\ncompleted,100,EUR,0.002,BTC,2025-01-02,atomic\n").unwrap();
        let result = execute(
            &conn,
            Request {
                profile_id: "p".into(),
                wallet: "Source".into(),
                file: file.to_string_lossy().into_owned(),
                format: Format::Bull,
                mode: Some("full".into()),
            },
            None,
        );
        std::fs::remove_file(file).unwrap();
        assert!(result.is_err(), "{step}");
        for table in [
            "transactions",
            "tags",
            "transaction_tags",
            "import_batches",
            "import_batch_transactions",
        ] {
            assert_eq!(
                scalar(&conn, &format!("SELECT count(*) FROM {table}")),
                0,
                "{step}: {table}"
            );
        }
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            0,
            "{step}"
        );
        assert!(conn.is_autocommit());
    }
}
#[test]
fn bull_import_mode_aliases_and_refusals() {
    for mode in [
        None,
        Some(""),
        Some("relevant"),
        Some("ReLeVaNt"),
        Some("match"),
        Some("match-only"),
        Some("match_only"),
        Some("relevant-only"),
        Some("relevant_only"),
    ] {
        assert_eq!(exchange::mode(mode).unwrap(), "relevant");
    }
    assert_eq!(exchange::mode(Some(" FULL\u{a0}")).unwrap(), "full");
    let error = exchange::mode(Some("bad")).unwrap_err();
    assert_eq!(error.code, "validation");
    assert_eq!(
        error.message,
        "Unsupported exchange-evidence import mode 'bad'"
    );
    assert!(error.hint.unwrap().contains("Choose 'relevant'"));
}

#[test]
fn bull_relevant_validates_only_matched_prices_and_preserves_committed_raw() {
    let conn = fixture();
    let result = bull(&conn, "relevant"); // EUR evidence is unmatched in a USD book.
    assert_eq!(
        kassiber_envelope::get(&result, "skipped_unmatched"),
        Some(&Py::from(1))
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        0
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
    import(&conn, "date,txid,amount\n2025-01-01,match,0.002\n").unwrap();
    let rows=bull::parse(csv::rows("ORDER_STATUS,PAYIN_AMOUNT,PAYIN_CURRENCY,PAYOUT_AMOUNT,PAYOUT_CURRENCY,COMPLETED_AT (UTC),TRANSACTION_ID\ncompleted,100,EUR,0.002,BTC,2025-01-02,match\n").unwrap()).unwrap();
    let request = Request {
        profile_id: "p".into(),
        wallet: "Source".into(),
        file: "bull.csv".into(),
        format: Format::Bull,
        mode: None,
    };
    let error = exchange::enrich(&conn, &request, &rows).unwrap_err();
    assert_eq!(error.code, "validation");
    assert_eq!(
        error.message,
        "Imported price currency EUR does not match profile fiat currency USD"
    );
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    let raw = r#"{"observer":"synthetic"}"#;
    conn.execute("UPDATE transactions SET raw_json=?,kind='authored',description='authored',counterparty='authored',note='authored'",[raw]).unwrap();
    let id: String = conn
        .query_row("SELECT id FROM transactions", [], |r| r.get(0))
        .unwrap();
    let value = json!({"wallet_id":"wallet","external_id":"match","direction":"inbound","asset":"BTC","amount":200000000_i64,"fee":0,"amount_includes_fee":0});
    let graph = kassiber_custody::evidence::graph_hash(&json!(raw))
        .unwrap()
        .0;
    let quantity = kassiber_custody::evidence::observed_hash(&value).unwrap().0;
    conn.execute("INSERT INTO chain_observation_provenance(transaction_id,workspace_id,profile_id,wallet_id,authority_version,observer_ids_json,observer_kinds_json,chain,network,application_revision,graph_hash,quantity_hash,fee_attribution,observed_at,updated_at) VALUES(?,'ws','p','wallet',1,'[]','[]','bitcoin','main','r',?,?,'exact','2025-01-01','2025-01-01')",params![id,graph,quantity]).unwrap();
    bull(&conn, "relevant");
    let stored: String = conn
        .query_row("SELECT raw_json FROM transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, raw);
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE kind='authored' AND description='authored' AND counterparty='authored' AND note='authored' AND fiat_value_exact='100'"),1);
    conn.execute(
        "UPDATE transactions SET pricing_source_kind='manual_override'",
        [],
    )
    .unwrap();
    let version = scalar(
        &conn,
        "SELECT journal_input_version FROM profiles WHERE id='p'",
    );
    let unchanged = bull(&conn, "relevant");
    assert_eq!(
        kassiber_envelope::get(&unchanged, "unchanged"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&unchanged, "journal_invalidated"),
        Some(&Py::Bool(false))
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        version
    );
}

#[test]
fn bull_default_wallet_admission_precedes_entropy_and_commits_before_file_read() {
    let conn = fixture();
    let request = || Request {
        profile_id: "p".into(),
        wallet: String::new(),
        file: std::env::temp_dir()
            .join("missing-bull-input.csv")
            .to_string_lossy()
            .into_owned(),
        format: Format::Bull,
        mode: Some("full".into()),
    };
    let identity = Identity {
        ids: vec![],
        timestamp: "2025-01-01T00:00:00Z".into(),
    };
    let error = execute(&conn, request(), Some(&identity)).unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("treasury"), "{error:?}");
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 1);
    conn.execute_batch("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','2025-01-01')").unwrap();
    let identity = Identity {
        ids: vec!["auto-wallet".into()],
        timestamp: "2025-01-01T00:00:00Z".into(),
    };
    let error = execute(&conn, request(), Some(&identity)).unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("Import file not found"));
    assert_eq!(scalar(&conn,"SELECT count(*) FROM wallets WHERE id='auto-wallet' AND kind='bullbitcoin' AND label='Bull Bitcoin' AND account_id='treasury'"),1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
    // Existing default labels are reused, even when their kind is custom.
    conn.execute(
        "UPDATE wallets SET label='bull bitcoin',kind='custom' WHERE id='auto-wallet'",
        [],
    )
    .unwrap();
    let identity = Identity {
        ids: vec![],
        timestamp: "2025-01-01T00:00:00Z".into(),
    };
    let error = execute(&conn, request(), Some(&identity)).unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("Import file not found"));
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 2);
}

#[test]
fn bull_relevant_preserves_reference_error_order_and_rollback() {
    let conn = fixture();
    import(&conn, "date,txid,amount\n2025-01-01,match,0.002\n").unwrap();
    let version = scalar(
        &conn,
        "SELECT journal_input_version FROM profiles WHERE id='p'",
    );
    let file = std::env::temp_dir().join(format!("bull-order-{}.csv", std::process::id()));
    std::fs::write(&file,"ORDER_STATUS,PAYIN_AMOUNT,PAYIN_CURRENCY,PAYOUT_AMOUNT,PAYOUT_CURRENCY,COMPLETED_AT (UTC),TRANSACTION_ID\ncompleted,100,EUR,0.002,BTC,2025-01-02,match\ncompleted,100,USD,0.002,BTC,invalid,later\n").unwrap();
    let error = execute(
        &conn,
        Request {
            profile_id: "p".into(),
            wallet: "Source".into(),
            file: file.to_string_lossy().into_owned(),
            format: Format::Bull,
            mode: Some("relevant".into()),
        },
        None,
    )
    .unwrap_err();
    std::fs::remove_file(file).unwrap();
    assert_eq!(error.code, "validation");
    assert_eq!(
        error.message,
        "Imported price currency EUR does not match profile fiat currency USD"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        version
    );
    assert!(conn.is_autocommit());
}

fn coinfinity(conn: &Connection, mode: &str) -> Py {
    let rows = coinfinity::parse(csv::rows("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,0,match\n").unwrap()).unwrap();
    let records = rows
        .iter()
        .map(normalize)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let request = Request {
        profile_id: "p".into(),
        wallet: "Source".into(),
        file: "coinfinity.csv".into(),
        format: Format::Coinfinity,
        mode: Some(mode.into()),
    };
    if mode == "relevant" {
        exchange::enrich(conn, &request, &rows).unwrap()
    } else {
        apply(
            conn,
            &request,
            &resolve(conn, "p", "Source").unwrap(),
            &rows,
            &records,
            &mut Ids {
                identity: None,
                at: 0,
            },
            None,
        )
        .unwrap()
    }
}

#[test]
fn coinfinity_full_exclusion_reconciliation_dedupe_and_readiness() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    let first = coinfinity(&conn, "full");
    assert_eq!(
        kassiber_envelope::get(&first, "coinfinity_rows"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&first, "unmatched"),
        Some(&Py::from(1))
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transactions WHERE excluded=1"),
        1
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM tags"), 3);
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='coinfinity-wallet-gap' AND label='Coinfinity wallet gap'"),1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        2
    );
    let repeat = coinfinity(&conn, "full");
    assert_eq!(
        kassiber_envelope::get(&repeat, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(
        kassiber_envelope::get(&repeat, "updated"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&repeat, "unmatched"),
        Some(&Py::from(1)),
        "exclude destination wallet before insertion"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        4
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='q'"
        ),
        0
    );
    conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('other','ws','p','Other','custom','{}','2025-01-01');
        INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('observed','ws','p','other','match','o','2025-01-01','inbound','BTC',200000000,0,'2025-01-01');").unwrap();
    let matched = coinfinity(&conn, "full");
    assert_eq!(
        kassiber_envelope::get(&matched, "matched"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='coinfinity-matched' AND label='Coinfinity matched'"),1);
    conn.execute_batch("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('sibling','ws','p','other','match','s','2025-01-01','inbound','BTC',200000000,0,'2025-01-01');").unwrap();
    let ambiguous = coinfinity(&conn, "full");
    assert_eq!(
        kassiber_envelope::get(&ambiguous, "ambiguous"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='coinfinity-ambiguous' AND label='Coinfinity ambiguous'"),1);
}

#[test]
fn coinfinity_coordinator_failure_rolls_back_transactions_flags_run_and_invalidation() {
    for step in ["tags", "transaction_tags", "import_batches"] {
        let conn = fixture();
        conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
            .unwrap();
        conn.execute_batch(&format!(
            "CREATE TRIGGER deny BEFORE INSERT ON {step} BEGIN SELECT RAISE(ABORT,'denied');END;"
        ))
        .unwrap();
        let file = std::env::temp_dir().join(format!(
            "coinfinity-atomic-{}-{step}.csv",
            std::process::id()
        ));
        std::fs::write(&file,"Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,0,atomic\n").unwrap();
        let result = execute(
            &conn,
            Request {
                profile_id: "p".into(),
                wallet: "Source".into(),
                file: file.to_string_lossy().into_owned(),
                format: Format::Coinfinity,
                mode: Some("full".into()),
            },
            None,
        );
        std::fs::remove_file(file).unwrap();
        assert!(result.is_err(), "{step}");
        for table in [
            "transactions",
            "tags",
            "transaction_tags",
            "import_batches",
            "import_batch_transactions",
        ] {
            assert_eq!(
                scalar(&conn, &format!("SELECT count(*) FROM {table}")),
                0,
                "{step}: {table}"
            );
        }
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            0,
            "{step}"
        );
        assert!(conn.is_autocommit());
    }
}

#[test]
fn coinfinity_economic_matching_window_scope_exclusion_and_ambiguity() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    import(
        &conn,
        "date,txid,amount,fee\n2025-01-02,observed,0.002,0.00001\n",
    )
    .unwrap();
    let rows = coinfinity::parse(csv::rows("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,1,\n").unwrap()).unwrap();
    let record = normalize(&rows[0]).unwrap();
    let find = |exclude| exchange::find_profile(&conn, "p", &record, exclude).unwrap();
    assert_eq!(find(None).1, "matched");
    assert_eq!(find(Some("wallet")).1, "unmatched");
    for (stamp, status) in [
        ("2024-12-30T23:59:59Z", "unmatched"),
        ("2024-12-31T00:00:00Z", "matched"),
        ("2025-01-04T00:00:00Z", "matched"),
        ("2025-01-04T00:00:01Z", "unmatched"),
    ] {
        conn.execute("UPDATE transactions SET occurred_at=?", [stamp])
            .unwrap();
        assert_eq!(find(None).1, status, "{stamp}");
    }
    conn.execute(
        "UPDATE transactions SET occurred_at='2025-01-02T00:00:00Z'",
        [],
    )
    .unwrap();
    for (column, different, original) in [
        ("profile_id", "q", "p"),
        ("direction", "outbound", "inbound"),
        ("asset", "LBTC", "BTC"),
        ("amount", "1", "200000000"),
    ] {
        conn.execute(&format!("UPDATE transactions SET {column}=?"), [different])
            .unwrap();
        assert_eq!(find(None).1, "unmatched", "{column}");
        conn.execute(&format!("UPDATE transactions SET {column}=?"), [original])
            .unwrap();
    }
    conn.execute(
        "UPDATE transactions SET excluded=1,pricing_method='coinfinity_csv'",
        [],
    )
    .unwrap();
    assert_eq!(find(None).1, "unmatched");
    conn.execute("UPDATE transactions SET pricing_method='other'", [])
        .unwrap();
    assert_eq!(find(None).1, "matched");
    conn.execute(
        "UPDATE transactions SET excluded=0,pricing_method='coinfinity_csv'",
        [],
    )
    .unwrap();
    assert_eq!(find(None).1, "matched");
    let request = Request {
        profile_id: "p".into(),
        wallet: String::new(),
        file: "coinfinity.csv".into(),
        format: Format::Coinfinity,
        mode: None,
    };
    let result = exchange::enrich(&conn, &request, &rows).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "matched"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE fee=1000000 AND fiat_value_exact='101' AND external_id='observed' AND kind='buy'"),1);
    conn.execute_batch("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('sibling','ws','p','wallet','other','s','2025-01-01T00:00:00Z','inbound','BTC',200000000,0,'2025-01-01');").unwrap();
    assert_eq!(find(None).1, "ambiguous");
    assert_eq!(
        kassiber_envelope::get(
            &exchange::enrich(&conn, &request, &rows).unwrap(),
            "skipped_ambiguous"
        ),
        Some(&Py::from(1))
    );
    // A declared transaction or invoice never falls back to economic matching.
    let mut identified = record;
    identified.economics = false;
    assert_eq!(
        exchange::find_profile(&conn, "p", &identified, None)
            .unwrap()
            .1,
        "unmatched"
    );
}

#[test]
fn coinfinity_default_wallet_admission_precedes_entropy_and_commits_before_file_read() {
    let conn = fixture();
    let request = || Request {
        profile_id: "p".into(),
        wallet: String::new(),
        file: std::env::temp_dir()
            .join("missing-coinfinity-input.csv")
            .to_string_lossy()
            .into_owned(),
        format: Format::Coinfinity,
        mode: Some("full".into()),
    };
    let identity = Identity {
        ids: vec![],
        timestamp: "2025-01-01T00:00:00Z".into(),
    };
    let error = execute(&conn, request(), Some(&identity)).unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("treasury"), "{error:?}");
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 1);
    conn.execute_batch("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','2025-01-01')").unwrap();
    let identity = Identity {
        ids: vec!["auto-wallet".into()],
        timestamp: "2025-01-01T00:00:00Z".into(),
    };
    let error = execute(&conn, request(), Some(&identity)).unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("Import file not found"));
    assert_eq!(scalar(&conn,"SELECT count(*) FROM wallets WHERE id='auto-wallet' AND kind='coinfinity' AND label='Coinfinity' AND account_id='treasury'"),1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
    // Existing default labels are reused, even when their kind is custom.
    conn.execute(
        "UPDATE wallets SET label='coinfinity',kind='custom' WHERE id='auto-wallet'",
        [],
    )
    .unwrap();
    let identity = Identity {
        ids: vec![],
        timestamp: "2025-01-01T00:00:00Z".into(),
    };
    let error = execute(&conn, request(), Some(&identity)).unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("Import file not found"));
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 2);
}

#[test]
fn coinfinity_economic_window_preserves_datetime_overflow() {
    let conn = fixture();
    let rows = coinfinity::parse(csv::rows("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,1,\n").unwrap()).unwrap();
    let mut r = normalize(&rows[0]).unwrap();
    for date in ["0001-01-01T00:00:00Z", "9999-12-31T23:59:59Z"] {
        r.fields
            .insert("occurred_at".into(), Sql::Text(date.into()));
        let error = exchange::find_profile(&conn, "p", &r, None).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("internal_error", "date value out of range")
        );
    }
    assert_eq!(
        crate::time::shifted("0001-01-03T00:00:00Z", -172800).unwrap(),
        "0001-01-01T00:00:00Z"
    );
    assert_eq!(
        crate::time::shifted("9999-12-29T23:59:59Z", 172800).unwrap(),
        "9999-12-31T23:59:59Z"
    );
}

#[test]
fn coinfinity_declared_references_disable_economic_matching() {
    let conn = fixture();
    import(&conn, "date,txid,amount\n2025-01-02,observed,0.002\n").unwrap();
    for (transaction, invoice) in [
        ("declared", ""),
        ("", "lnbc1declared"),
        ("declared", "lnbc1declared"),
    ] {
        let input=format!("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction,LN Invoice\norder,sell,2025-01-02,100,0.002,BTC,,0,1,{transaction},{invoice}\n");
        let rows = coinfinity::parse(csv::rows(&input).unwrap()).unwrap();
        let r = normalize(&rows[0]).unwrap();
        assert!(!r.economics);
        assert_eq!(
            exchange::find_profile(&conn, "p", &r, None).unwrap().1,
            "unmatched"
        );
    }
}

#[test]
fn coinfinity_economic_matching_preserves_closed_observation() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    import(
        &conn,
        "date,txid,amount,fee\n2025-01-02,observed,0.002,0.00001\n",
    )
    .unwrap();
    let raw = r#"{"observer":"synthetic"}"#;
    conn.execute("UPDATE transactions SET raw_json=?,kind='authored',description='authored',counterparty='authored'",[raw]).unwrap();
    let id: String = conn
        .query_row("SELECT id FROM transactions", [], |r| r.get(0))
        .unwrap();
    let value = json!({"wallet_id":"wallet","external_id":"observed","direction":"inbound","asset":"BTC","amount":200000000_i64,"fee":1000000,"amount_includes_fee":0});
    let graph = kassiber_custody::evidence::graph_hash(&json!(raw))
        .unwrap()
        .0;
    let quantity = kassiber_custody::evidence::observed_hash(&value).unwrap().0;
    conn.execute("INSERT INTO chain_observation_provenance(transaction_id,workspace_id,profile_id,wallet_id,authority_version,observer_ids_json,observer_kinds_json,chain,network,application_revision,graph_hash,quantity_hash,fee_attribution,observed_at,updated_at) VALUES(?,'ws','p','wallet',1,'[]','[]','bitcoin','main','r',?,?,'exact','2025-01-01','2025-01-01')",params![id,graph,quantity]).unwrap();
    let rows=coinfinity::parse(csv::rows("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,1,\n").unwrap()).unwrap();
    let request = Request {
        profile_id: "p".into(),
        wallet: String::new(),
        file: "coinfinity.csv".into(),
        format: Format::Coinfinity,
        mode: None,
    };
    let result = exchange::enrich(&conn, &request, &rows).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "matched"),
        Some(&Py::from(1))
    );
    let stored: String = conn
        .query_row("SELECT raw_json FROM transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, raw);
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE kind='authored' AND description='authored' AND counterparty='authored' AND fiat_value_exact='101' AND fee=1000000"),1);
}

const TWENTYONE_HEADER: &str = "id,transaction_date,buy_asset,buy_amount,sell_asset,sell_amount,fee_asset,fee_amount,transaction_type,note\n";
const TWENTYONE_BUY: &str = "one,01.01.222 01:02:03,BTC,0.002,EUR,100,EUR,1,trade,purchase\n";
#[test]
fn twentyone_active_ledger_dedupe_updates_legacy_flags_runs_and_readiness() {
    for encrypted in [false, true] {
        let conn = fixture_codec(encrypted);
        conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
            .unwrap();
        let first = format!("{TWENTYONE_HEADER}{TWENTYONE_BUY}");
        let result = import_kind(&conn, &first, Format::TwentyOne).unwrap();
        assert_eq!(kassiber_envelope::get(&result, "mode"), Some(&s("full")));
        assert_eq!(
            kassiber_envelope::get(&result, "twentyonebitcoin_rows"),
            Some(&Py::from(1))
        );
        assert_eq!(scalar(&conn, "SELECT excluded FROM transactions"), 0);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM tags"), 0);
        conn.execute_batch("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('legacy','ws','p','21bitcoin-wallet-gap','old','2025-01-01'),('authored','ws','p','keep','Authored','2025-01-01'),('legacy2','ws','p','21bitcoin-matched','old matched','2025-01-01'),('legacy3','ws','p','21bitcoin-ambiguous','old ambiguous','2025-01-01'); INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy' FROM transactions; INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy2' FROM transactions; INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy3' FROM transactions; INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'authored' FROM transactions; UPDATE transactions SET excluded=1;").unwrap();
        let second = format!(
            "{TWENTYONE_HEADER}{}two,02.01.2022,,,BTC,0.001,BTC,0.00001,withdrawal,send\n",
            TWENTYONE_BUY.replace("purchase", "updated")
        );
        let result = import_kind(&conn, &second, Format::TwentyOne).unwrap();
        for (key, count) in [
            ("imported", 1),
            ("updated", 1),
            ("twentyonebitcoin_rows", 2),
            ("reconciliation_flags_cleared", 1),
            ("reactivated", 1),
        ] {
            assert_eq!(
                kassiber_envelope::get(&result, key),
                Some(&Py::from(count)),
                "{key}"
            );
        }
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transaction_tags WHERE tag_id LIKE 'legacy%'"
            ),
            0
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transaction_tags WHERE tag_id='authored'"
            ),
            1
        );
        assert_eq!(scalar(&conn, "SELECT sum(excluded) FROM transactions"), 0);
        assert_eq!(
            scalar(&conn, "SELECT sum(rows_updated) FROM import_batches"),
            1
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 2);
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            2
        );
        let result = import_kind(&conn, &second, Format::TwentyOne).unwrap();
        assert_eq!(
            kassiber_envelope::get(&result, "unchanged"),
            Some(&Py::from(1))
        );
        assert_eq!(
            kassiber_envelope::get(&result, "updated"),
            Some(&Py::from(1))
        );
        assert_eq!(kassiber_envelope::get(&result, "reactivated"), None);
        assert_eq!(
            kassiber_envelope::get(&result, "journal_invalidated"),
            Some(&Py::Bool(true))
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 2);
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE asset='BTC' AND direction='outbound' AND fee=1000000"),1);
    }
}
#[test]
fn twentyone_refusal_precedes_wallet_and_file_and_default_wallet_commits_before_read() {
    let conn = fixture();
    let request = |mode| Request {
        profile_id: "p".into(),
        wallet: String::new(),
        file: "missing-21bitcoin-input.csv".into(),
        format: Format::TwentyOne,
        mode,
    };
    let error = execute(
        &conn,
        request(Some("relevant".into())),
        Some(&Identity {
            ids: vec![],
            timestamp: "2025-01-01".into(),
        }),
    )
    .unwrap_err();
    assert_eq!(
        (&*error.code, &*error.message),
        (
            "validation",
            "21bitcoin_csv is an active custodial ledger import"
        )
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("Import the complete platform ledger, then review custody-transfer candidates.")
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 1);
    conn.execute_batch("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','2025-01-01')").unwrap();
    let error = execute(
        &conn,
        request(None),
        Some(&Identity {
            ids: vec!["provider".into()],
            timestamp: "2025-01-01".into(),
        }),
    )
    .unwrap_err();
    assert_eq!(error.code, "not_found");
    let stored: (String, String, String) = conn
        .query_row(
            "SELECT label,kind,config_json FROM wallets WHERE id='provider'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        (
            "21bitcoin".into(),
            "21bitcoin".into(),
            r#"{"source_format": "21bitcoin_csv"}"#.into()
        )
    );
    assert!(conn.is_autocommit());
}
#[test]
fn twentyone_batch_failure_rolls_back_observations_legacy_flags_and_readiness() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    let first = format!("{TWENTYONE_HEADER}{TWENTYONE_BUY}");
    import_kind(&conn, &first, Format::TwentyOne).unwrap();
    conn.execute_batch("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('legacy','ws','p','21bitcoin-matched','old','2025-01-01'); INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy' FROM transactions; UPDATE transactions SET excluded=1; CREATE TRIGGER refuse_21_batch BEFORE INSERT ON import_batches BEGIN SELECT RAISE(ABORT,'batch denied');END;").unwrap();
    let content = format!("{first}two,02.01.2022,BTC,0.001,,,BTC,0,deposit,receive\n");
    assert!(import_kind(&conn, &content, Format::TwentyOne).is_err());
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    assert_eq!(scalar(&conn, "SELECT excluded FROM transactions"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
    assert!(conn.is_autocommit());
}

#[test]
fn twentyone_legacy_fallback_preserves_scope_identity_and_unrelated_exclusions() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    let first = format!("{TWENTYONE_HEADER}{TWENTYONE_BUY}");
    import_kind(&conn, &first, Format::TwentyOne).unwrap();
    conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('other','ws','p','Other','custom','{}','2025-01-01'); INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('legacy','ws','p','21bitcoin-matched','old','2025-01-01'); UPDATE transactions SET external_id='old-provider-id',fingerprint='old-fingerprint',excluded=1;").unwrap();
    for (id, wallet, direction, asset, amount) in [
        ("wrong-wallet", "other", "inbound", "BTC", 200000000_i64),
        ("wrong-direction", "wallet", "outbound", "BTC", 200000000),
        ("wrong-asset", "wallet", "inbound", "LBTC", 200000000),
        ("wrong-amount", "wallet", "inbound", "BTC", 200000001),
    ] {
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,pricing_external_ref,excluded,created_at) VALUES(?,'ws','p',?,'',?,'2022-01-01',?,?,?,0,'one',1,'9999-01-01')",params![id,wallet,id,direction,asset,amount]).unwrap();
    }
    conn.execute_batch(
        "INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy' FROM transactions;",
    )
    .unwrap();
    let rows = twentyone::parse(csv::rows(&first).unwrap()).unwrap();
    let mut record = normalize(&rows[0]).unwrap();
    let request = Request {
        profile_id: "p".into(),
        wallet: "Source".into(),
        file: "unused".into(),
        format: Format::TwentyOne,
        mode: None,
    };
    let wallet = resolve(&conn, "p", "Source").unwrap();
    let mut outcome = Vec::new();
    exchange::clear_legacy_flags(
        &conn,
        &request,
        &wallet,
        std::slice::from_ref(&record),
        &mut outcome,
    )
    .unwrap();
    assert_eq!(
        outcome,
        vec![
            ("reconciliation_flags_cleared", Py::from(1)),
            ("reactivated", Py::from(1))
        ]
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transactions WHERE excluded=1"),
        4
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT excluded FROM transactions WHERE external_id='old-provider-id'"
        ),
        0
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE id LIKE 'wrong-%' AND excluded=1"
        ),
        4
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 4);
    record
        .fields
        .insert("pricing_external_ref".into(), Sql::Text("absent".into()));
    let mut outcome = Vec::new();
    exchange::clear_legacy_flags(
        &conn,
        &request,
        &wallet,
        std::slice::from_ref(&record),
        &mut outcome,
    )
    .unwrap();
    assert!(outcome.is_empty());
}

#[test]
fn python_pocket_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_pocket_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

const POCKET_HEADER: &str = "type,date,reference,price.currency,price.amount,cost.currency,cost.amount,fee.currency,fee.amount,value.currency,value.amount\n";
const POCKET_PAIR: &str = "exchange,2025-01-02,order,EUR,,EUR,100,EUR,1.2,BTC,0.002\nwithdrawal,2025-01-03,withdraw,EUR,,EUR,0,BTC,0.00001,BTC,0.00199\n";
fn pocket(conn: &Connection, mode: &str) -> Py {
    let rows = pocket::parse(csv::rows(&format!("{POCKET_HEADER}{POCKET_PAIR}")).unwrap()).unwrap();
    let records = rows
        .iter()
        .map(normalize)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let request = Request {
        profile_id: "p".into(),
        wallet: "Source".into(),
        file: "pocket.csv".into(),
        format: Format::Pocket,
        mode: Some(mode.into()),
    };
    if mode == "relevant" {
        exchange::enrich(conn, &request, &rows).unwrap()
    } else {
        apply(
            conn,
            &request,
            &resolve(conn, "p", "Source").unwrap(),
            &rows,
            &records,
            &mut Ids {
                identity: None,
                at: 0,
            },
            None,
        )
        .unwrap()
    }
}
#[test]
fn pocket_paired_economics_full_flags_dedupe_and_observation_authority() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('observed','ws','p','Observed','custom','{}','2025-01-01'); INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('observed','ws','p','observed','chain','observed','2025-01-02T00:00:00Z','inbound','BTC',199000000,2000000,'2025-01-01');").unwrap();
    let relevant = pocket(&conn, "relevant");
    assert_eq!(
        kassiber_envelope::get(&relevant, "matched"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    let ready = scalar(
        &conn,
        "SELECT journal_input_version FROM profiles WHERE id='p'",
    );
    assert_eq!(ready, 1);
    let full = pocket(&conn, "full");
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        ready + 2
    );
    assert_eq!(kassiber_envelope::get(&full, "matched"), Some(&Py::from(1)));
    assert_eq!(
        kassiber_envelope::get(&full, "pocketbitcoin_rows"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM tags WHERE code IN ('pocketbitcoin-matched','pocketbitcoin-wallet-gap','pocketbitcoin-ambiguous') AND label LIKE 'Pocket Bitcoin %'"),3);
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='pocketbitcoin-matched'"),1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT excluded FROM transactions WHERE wallet_id='wallet'"
        ),
        1
    );
    assert_eq!(
        scalar(&conn, "SELECT fee FROM transactions WHERE id='observed'"),
        2000000
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT amount FROM transactions WHERE wallet_id='wallet'"
        ),
        199000000
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT fee FROM transactions WHERE wallet_id='wallet'"
        ),
        1000000
    );
    let repeat = pocket(&conn, "full");
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        ready + 4
    );
    assert_eq!(
        kassiber_envelope::get(&repeat, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM import_batch_transactions"),
        1
    );
}
#[test]
fn pocket_default_wallet_admission_precedes_entropy_and_file_access() {
    let conn = fixture();
    conn.execute_batch("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','2025-01-01')").unwrap();
    let request = || Request {
        profile_id: "p".into(),
        wallet: String::new(),
        file: std::env::temp_dir()
            .join("missing-pocket-input.csv")
            .to_string_lossy()
            .into(),
        format: Format::Pocket,
        mode: Some("full".into()),
    };
    let identity = Identity {
        ids: vec!["provider".into()],
        timestamp: "2025-01-01T00:00:00Z".into(),
    };
    let error = execute(&conn, request(), Some(&identity)).unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("Import file not found"));
    assert_eq!(scalar(&conn,"SELECT count(*) FROM wallets WHERE id='provider' AND label='Pocket Bitcoin' AND kind='pocketbitcoin' AND account_id='treasury'"),1);
    let error = execute(
        &conn,
        request(),
        Some(&Identity {
            ids: vec![],
            timestamp: "2025-01-01T00:00:00Z".into(),
        }),
    )
    .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("Import file not found"));
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 2);
}
#[test]
fn pocket_late_coordinator_failure_rolls_back_flags_run_and_invalidation() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    conn.execute_batch("CREATE TRIGGER refuse_pocket BEFORE INSERT ON import_batches BEGIN SELECT RAISE(ABORT,'pocket denied');END;").unwrap();
    let file = std::env::temp_dir().join(format!("pocket-rollback-{}.csv", std::process::id()));
    std::fs::write(&file, format!("{POCKET_HEADER}{POCKET_PAIR}")).unwrap();
    let result = execute(
        &conn,
        Request {
            profile_id: "p".into(),
            wallet: "Source".into(),
            file: file.to_string_lossy().into(),
            format: Format::Pocket,
            mode: Some("full".into()),
        },
        None,
    );
    std::fs::remove_file(file).unwrap();
    assert!(result.is_err());
    for table in [
        "transactions",
        "tags",
        "transaction_tags",
        "import_batches",
        "import_batch_transactions",
    ] {
        assert_eq!(
            scalar(&conn, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        0
    );
    assert!(conn.is_autocommit());
}

#[test]
fn pocket_reimport_matches_provider_identity_when_fee_changes() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    pocket(&conn, "full");
    let changed = format!(
        "{POCKET_HEADER}{}",
        POCKET_PAIR
            .replace("0.002\n", "0.00201\n")
            .replace("0.00001,BTC", "0.00002,BTC")
    );
    let rows = pocket::parse(csv::rows(&changed).unwrap()).unwrap();
    let records = rows
        .iter()
        .map(normalize)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let request = Request {
        profile_id: "p".into(),
        wallet: "Source".into(),
        file: "pocket.csv".into(),
        format: Format::Pocket,
        mode: Some("full".into()),
    };
    let result = apply(
        &conn,
        &request,
        &resolve(&conn, "p", "Source").unwrap(),
        &rows,
        &records,
        &mut Ids {
            identity: None,
            at: 0,
        },
        None,
    )
    .unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "updated"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    assert_eq!(scalar(&conn, "SELECT fee FROM transactions"), 1000000);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
}

#[test]
fn python_strike_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_strike_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

#[test]
fn python_ledgerlive_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_ledgerlive_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

const LEDGERLIVE_HEADER: &str = "Operation Date,Currency Ticker,Operation Type,Operation Amount,Operation Fees,Operation Hash,Account Name,Account xpub,Countervalue Ticker,Countervalue at Operation Date\n";
const LEDGERLIVE_OUT: &str = "2025-01-02T01:02:03+01:00,BTC,OUT,-0.00200000,0.00001000,ledger-out,Bitcoin,xpub-synthetic-secret,EUR,100\n";

#[test]
fn ledgerlive_movement_dedupe_runs_updates_and_readiness() {
    let conn = fixture();
    let first = format!("{LEDGERLIVE_HEADER}{LEDGERLIVE_OUT}");
    let result = import_kind(&conn, &first, Format::LedgerLive).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "source"),
        Some(&s("file:ledgerlive_csv"))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions WHERE external_id='ledger-out' AND direction='outbound' AND kind='withdrawal' AND amount=200000000 AND fee=1000000 AND occurred_at='2025-01-02T00:02:03Z' AND excluded=0 AND fiat_value IS NULL AND pricing_source_kind IS NULL"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions WHERE raw_json LIKE '%[redacted]%' AND raw_json NOT LIKE '%xpub-synthetic-secret%'"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches WHERE source_format='ledgerlive_csv' AND rows_inserted=1"), 1);
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM import_batch_transactions"),
        1
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
    let repeated = import_kind(&conn, &first, Format::LedgerLive).unwrap();
    assert_eq!(
        kassiber_envelope::get(&repeated, "unchanged"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&repeated, "journal_invalidated"),
        Some(&Py::Bool(false))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
    conn.execute("UPDATE transactions SET description=NULL", [])
        .unwrap();
    import_kind(
        &conn,
        &first.replace(",Bitcoin,", ",Updated,"),
        Format::LedgerLive,
    )
    .unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        2
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions WHERE description='Ledger Live withdrawal - Updated'"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM tags"), 0);
}

#[test]
fn ledgerlive_refusals_and_late_failure_leave_the_book_unchanged() {
    let conn = fixture();
    let error = execute(
        &conn,
        Request {
            profile_id: "p".into(),
            wallet: "missing".into(),
            file: "missing-ledgerlive.csv".into(),
            format: Format::LedgerLive,
            mode: None,
        },
        None,
    )
    .unwrap_err();
    assert_eq!(
        (&*error.code, &*error.message),
        ("not_found", "Wallet 'missing' not found")
    );
    let error = import_kind(
        &conn,
        &format!("{LEDGERLIVE_HEADER}{LEDGERLIVE_OUT}invalid,BTC,IN,1,0,later,,,,\n"),
        Format::LedgerLive,
    )
    .unwrap_err();
    assert_eq!(error.code, "validation");
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
    conn.execute_batch("CREATE TRIGGER refuse_ledgerlive BEFORE INSERT ON import_batches BEGIN SELECT RAISE(ABORT,'ledgerlive denied');END;").unwrap();
    assert!(import_kind(
        &conn,
        &format!("{LEDGERLIVE_HEADER}{LEDGERLIVE_OUT}"),
        Format::LedgerLive
    )
    .is_err());
    for table in [
        "transactions",
        "import_batches",
        "import_batch_transactions",
    ] {
        assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
    }
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        0
    );
    assert!(conn.is_autocommit());
}

const STRIKE_HEADER: &str = "Reference,Date & Time (UTC),Transaction Type,Amount EUR,Fee EUR,Amount BTC,Fee BTC,BTC Price,Cost Basis (EUR),Destination,Description,Transaction Hash,Note\n";
const STRIKE_BUY: &str =
    "buy,Mar 08 2026 09:15:00,Buy,-100,1,0.001,0,,100,balance,purchase,,note\n";
#[test]
fn strike_active_ledger_dedupe_updates_identity_runs_and_readiness() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    let first = format!("{STRIKE_HEADER}{STRIKE_BUY}");
    let result = import_kind(&conn, &first, Format::Strike).unwrap();
    assert_eq!(kassiber_envelope::get(&result, "mode"), Some(&s("full")));
    assert_eq!(
        kassiber_envelope::get(&result, "strike_rows"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE external_id='strike:buy' AND excluded=0 AND amount=100000000 AND fee=0 AND kind='buy' AND fiat_value='101'"),1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM tags"), 0);
    let corrected_fee = first.replace("0.001,0,,100", "0.001,0.00001,,100");
    let unchanged = import_kind(&conn, &corrected_fee, Format::Strike).unwrap();
    assert_eq!(
        kassiber_envelope::get(&unchanged, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    assert_eq!(scalar(&conn, "SELECT fee FROM transactions"), 0);
    assert_eq!(
        kassiber_envelope::get(&unchanged, "updated"),
        Some(&Py::from(1)),
        "{unchanged:?}"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        2
    );
    let second=format!("{STRIKE_HEADER}{}ln,Mar 08 2026 10:00:00,Receive,,,0.002,0,,,lnbc1invoice,,{},lightning\nchain,Mar 09 2026 10:00:00,Send,,,-0.001,0.00001,60000,,bc1address,chain,{},withdrawal\n",STRIKE_BUY.replace("purchase","updated"),"ab".repeat(32),"cd".repeat(32));
    let result = import_kind(&conn, &second, Format::Strike).unwrap();
    for (key, value) in [("imported", 2), ("updated", 1), ("strike_rows", 3)] {
        assert_eq!(
            kassiber_envelope::get(&result, key),
            Some(&Py::from(value)),
            "{key}"
        );
    }
    assert_eq!(
        scalar(&conn, "SELECT sum(rows_updated) FROM import_batches"),
        1
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM import_batch_transactions"),
        3
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        3
    );
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE external_id='strike:ln' AND payment_hash IS NOT NULL AND payment_hash_source='importer' AND pricing_source_kind IS NULL AND excluded=0"),1);
    assert_eq!(scalar(&conn,&format!("SELECT count(*) FROM transactions WHERE external_id='{}' AND payment_hash IS NULL AND direction='outbound' AND fee=1000000 AND excluded=0","cd".repeat(32))),1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE raw_json LIKE '%_strike_lightning%'"
        ),
        3
    );
}
#[test]
fn strike_refusal_precedes_wallet_and_file_and_default_wallet_reuses() {
    let conn = fixture();
    let request = |mode| Request {
        profile_id: "p".into(),
        wallet: String::new(),
        file: "missing-strike-input.csv".into(),
        format: Format::Strike,
        mode,
    };
    let error = execute(&conn, request(Some("relevant".into())), None).unwrap_err();
    assert_eq!(
        (&*error.code, &*error.message),
        (
            "validation",
            "strike_csv is an active custodial ledger import"
        )
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("Import the complete platform ledger, then review custody-transfer candidates.")
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 1);
    conn.execute_batch("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','2025-01-01')").unwrap();
    let identity = Identity {
        ids: vec!["provider".into()],
        timestamp: "2025-01-01T00:00:00Z".into(),
    };
    let error = execute(&conn, request(None), Some(&identity)).unwrap_err();
    assert_eq!(error.code, "not_found");
    assert_eq!(scalar(&conn,"SELECT count(*) FROM wallets WHERE id='provider' AND label='Strike' AND kind='strike' AND account_id='treasury' AND config_json='{\"source_format\": \"strike_csv\"}'"),1);
    let error = execute(
        &conn,
        request(Some("full".into())),
        Some(&Identity {
            ids: vec![],
            timestamp: "2025-01-01".into(),
        }),
    )
    .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 2);
    assert!(conn.is_autocommit());
}
#[test]
fn strike_late_failure_rolls_back_observations_runs_and_readiness() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    conn.execute_batch("CREATE TRIGGER refuse_strike BEFORE INSERT ON import_batches BEGIN SELECT RAISE(ABORT,'strike denied');END;").unwrap();
    let result = import_kind(
        &conn,
        &format!("{STRIKE_HEADER}{STRIKE_BUY}"),
        Format::Strike,
    );
    assert!(result.is_err());
    for table in [
        "transactions",
        "import_batches",
        "import_batch_transactions",
    ] {
        assert_eq!(
            scalar(&conn, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        0
    );
    assert!(conn.is_autocommit());
}

#[test]
fn twentyone_legacy_reactivation_invalidates_ready_journals_once() {
    for encrypted in [false, true] {
        for (priced, excluded) in [(false, true), (false, false), (true, true)] {
            let conn = fixture_codec(encrypted);
            conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
                .unwrap();
            let row = if priced {
                TWENTYONE_BUY
            } else {
                "one,01.01.2022,,,BTC,0.002,BTC,0.0001,withdrawal,Withdrawal\n"
            };
            let content = format!("{TWENTYONE_HEADER}{row}");
            import_kind(&conn, &content, Format::TwentyOne).unwrap();
            if !priced {
                let stable = import_kind(&conn, &content, Format::TwentyOne).unwrap();
                assert_eq!(
                    kassiber_envelope::get(&stable, "journal_invalidated"),
                    Some(&Py::Bool(false))
                );
            }
            conn.execute_batch("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('legacy','ws','p','21bitcoin-matched','Old','2025-01-01'),('authored','ws','p','keep','Keep','2025-01-01'); INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy' FROM transactions; INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'authored' FROM transactions; UPDATE profiles SET last_processed_at='2026-01-11T00:00:00Z',last_processed_tx_count=1,last_processed_input_version=journal_input_version,ownership_review_counts_json='{}' WHERE id='p';").unwrap();
            conn.execute("UPDATE transactions SET excluded=?", [i64::from(excluded)])
                .unwrap();
            let before = scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'",
            );
            let result = import_kind(&conn, &content, Format::TwentyOne).unwrap();
            let invalidated = priced || excluded;
            assert_eq!(
                kassiber_envelope::get(&result, "journal_invalidated"),
                Some(&Py::Bool(invalidated))
            );
            assert_eq!(
                kassiber_envelope::get(&result, "reactivated"),
                excluded.then_some(Py::from(1)).as_ref()
            );
            assert_eq!(
                kassiber_envelope::get(&result, "reconciliation_flags_cleared"),
                Some(&Py::from(1))
            );
            assert_eq!(
                kassiber_envelope::get(&result, "imported"),
                Some(&Py::from(0))
            );
            assert_eq!(kassiber_envelope::get(&result, "import_batch_id"), None);
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT journal_input_version FROM profiles WHERE id='p'"
                ),
                before + i64::from(invalidated)
            );
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT last_processed_input_version FROM profiles WHERE id='p'"
                ),
                before
            );
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT last_processed_tx_count FROM profiles WHERE id='p'"
                ),
                i64::from(!invalidated)
            );
            assert_eq!(scalar(&conn, "SELECT count(*) FROM profiles WHERE id='p' AND last_processed_at IS NULL AND ownership_review_counts_json IS NULL"), i64::from(invalidated));
            assert_eq!(scalar(&conn, "SELECT excluded FROM transactions"), 0);
            assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 1);
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT count(*) FROM transaction_tags WHERE tag_id='authored'"
                ),
                1
            );
        }
    }
}

#[test]
fn twentyone_legacy_reactivation_rolls_back_when_invalidation_fails() {
    for encrypted in [false, true] {
        let conn = fixture_codec(encrypted);
        conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
            .unwrap();
        let content = format!(
            "{TWENTYONE_HEADER}one,01.01.2022,,,BTC,0.002,BTC,0.0001,withdrawal,Withdrawal\n"
        );
        import_kind(&conn, &content, Format::TwentyOne).unwrap();
        conn.execute_batch("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('legacy','ws','p','21bitcoin-matched','Old','2025-01-01'); INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy' FROM transactions; UPDATE transactions SET excluded=1; CREATE TRIGGER refuse_reactivation_invalidation BEFORE UPDATE OF journal_input_version ON profiles BEGIN SELECT RAISE(ABORT,'invalidation denied'); END;").unwrap();
        let error = import_kind(&conn, &content, Format::TwentyOne).unwrap_err();
        assert_eq!(error.code, "internal_error");
        assert_eq!(error.message, "database operation failed");
        assert_eq!(
            kassiber_envelope::get(error.details.as_ref().unwrap(), "sqlite_error_code"),
            Some(&Py::from(1811))
        );
        assert_eq!(scalar(&conn, "SELECT excluded FROM transactions"), 1);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 1);
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            1
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    }
}

#[test]
fn python_cointracking_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_cointracking_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

const COINTRACKING_HEADER: &str =
    "Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Date,Tx-ID,Comment\n";
fn cointracking_fixture(encrypted: bool) -> Connection {
    let conn = fixture_codec(encrypted);
    conn.execute(
        "UPDATE wallets SET kind='cointracking' WHERE id='wallet'",
        [],
    )
    .unwrap();
    conn
}
#[test]
fn cointracking_coordinator_reconciles_identity_economics_and_idless_collisions() {
    for encrypted in [false, true] {
        let conn = cointracking_fixture(encrypted);
        conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('chain','ws','p','Observed','descriptor','{}','2025-01-01');
            INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('economic','ws','p','chain','observed','economic','2025-01-02T00:00:00Z','inbound','BTC',200000000,0,'2025-01-01');").unwrap();
        let txid = "ab".repeat(32);
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('identity','ws','p','chain',?,'identity','2025-01-01T00:00:00Z','inbound','BTC',1,0,'2025-01-01')",[&txid]).unwrap();
        let body = format!("{COINTRACKING_HEADER}Deposit,0.002,BTC,,,,,2025-01-02,econ,one\nDeposit,0.003,BTC,,,,,2025-01-03,{txid},two\nDeposit,0.004,BTC,,,,,2025-01-04,,three\nDeposit,0.004,BTC,,,,,2025-01-04,,four\nDeposit,0.005,BTC,,,,,2025-01-05,active,five\n");
        let result = import_kind(&conn, &body, Format::CoinTracking).unwrap();
        for (key, n) in [
            ("imported", 5),
            ("matched", 1),
            ("ambiguous", 3),
            ("unmatched", 1),
            ("excluded", 4),
            ("reconciliation_rows", 5),
            ("reconciliation_changed", 4),
        ] {
            assert_eq!(
                kassiber_envelope::get(&result, key),
                Some(&Py::from(n)),
                "{key}"
            );
        }
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            1
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM tags"), 2);
        assert_eq!(
            scalar(&conn, "SELECT count(*) FROM import_batch_transactions"),
            5
        );
        assert_eq!(scalar(&conn,"SELECT count(*) FROM import_batches WHERE source_format='cointracking_csv' AND rows_inserted=5"),1);
        assert_eq!(scalar(&conn,"SELECT count(DISTINCT fingerprint) FROM transactions WHERE wallet_id='wallet' AND external_id IS NULL"),2);
        let repeated = import_kind(&conn, &body, Format::CoinTracking).unwrap();
        assert_eq!(
            kassiber_envelope::get(&repeated, "unchanged"),
            Some(&Py::from(5))
        );
        assert_eq!(
            kassiber_envelope::get(&repeated, "reconciliation_rows"),
            Some(&Py::from(0))
        );
        assert_eq!(
            kassiber_envelope::get(&repeated, "journal_invalidated"),
            Some(&Py::Bool(false))
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
        conn.execute("DELETE FROM transactions WHERE wallet_id='chain'", [])
            .unwrap();
        conn.execute("UPDATE transactions SET description=NULL WHERE wallet_id='wallet' AND external_id IS NOT NULL",[]).unwrap();
        let enriched = import_kind(&conn, &body, Format::CoinTracking).unwrap();
        assert_eq!(
            kassiber_envelope::get(&enriched, "reconciliation_flags_cleared"),
            Some(&Py::from(2))
        );
        assert_eq!(
            kassiber_envelope::get(&enriched, "reactivated"),
            Some(&Py::from(2))
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            2
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 2);
    }
}
#[test]
fn cointracking_admission_and_late_failure_leave_every_write_uncommitted() {
    let conn = fixture();
    let error = execute(
        &conn,
        Request {
            profile_id: "p".into(),
            wallet: "Source".into(),
            file: "missing.csv".into(),
            format: Format::CoinTracking,
            mode: None,
        },
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "migration_wallet_kind_required");
    assert_eq!(
        error.details,
        Some(dict([
            ("wallet", s("Source")),
            ("wallet_kind", s("custom"))
        ]))
    );
    for encrypted in [false, true] {
        let conn = cointracking_fixture(encrypted);
        let content = format!("{COINTRACKING_HEADER}Deposit,1,BTC,,,,,2025-01-02,,one\nDeposit,1,BTC,,,,,2025-01-02,,two\n");
        conn.execute_batch("CREATE TRIGGER refuse_cointracking BEFORE INSERT ON import_batches BEGIN SELECT RAISE(ABORT,'denied');END;").unwrap();
        assert!(import_kind(&conn, &content, Format::CoinTracking).is_err());
        for table in [
            "transactions",
            "tags",
            "transaction_tags",
            "import_batches",
            "import_batch_transactions",
        ] {
            assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
        }
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            0
        );
    }
}

#[test]
fn cointracking_authored_exclusions_survive_reconciliation_and_reactivation() {
    for encrypted in [false, true] {
        for (choice, expected) in [("true", 1), ("false", 0), ("null", 1)] {
            let conn = cointracking_fixture(encrypted);
            let body = format!("{COINTRACKING_HEADER}Deposit,1,BTC,,,,,2025-01-02,external,one\n");
            import_kind(&conn, &body, Format::CoinTracking).unwrap();
            conn.execute("INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,wallet_id,source,changed_at,journal_input_version_after) SELECT 'edit','ws','p',id,'wallet','test','2025-01-01',1 FROM transactions",[]).unwrap();
            conn.execute("INSERT INTO transaction_edit_fields(id,event_id,field,after_value) VALUES('field','edit','excluded',?)",[choice]).unwrap();
            conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('chain','ws','p','Observed','descriptor','2025-01-01');
            INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('economic','ws','p','chain','other','economic','2025-01-02T00:00:00Z','inbound','BTC',100000000000,0,'2025-01-01');
            UPDATE transactions SET description=NULL WHERE wallet_id='wallet';").unwrap();
            let flagged = import_kind(&conn, &body, Format::CoinTracking).unwrap();
            assert_eq!(
                kassiber_envelope::get(&flagged, "ambiguous"),
                Some(&Py::from(1))
            );
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT excluded FROM transactions WHERE wallet_id='wallet'"
                ),
                expected
            );
            conn.execute_batch("DELETE FROM transactions WHERE wallet_id='chain';UPDATE transactions SET description=NULL WHERE wallet_id='wallet';").unwrap();
            import_kind(&conn, &body, Format::CoinTracking).unwrap();
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT excluded FROM transactions WHERE wallet_id='wallet'"
                ),
                i64::from(choice == "true")
            );
            assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 0);
        }
    }
}

#[test]
fn cointracking_only_canonical_txids_establish_chain_identity() {
    for encrypted in [false, true] {
        for (external, copies, direction, asset, profile, matched, ambiguous) in [
            (
                format!("{}  {}", "ab".repeat(15), "cd".repeat(16)),
                1,
                "inbound",
                "BTC",
                "p",
                0,
                0,
            ),
            ("AB".repeat(32), 1, "inbound", "BTC", "p", 1, 0),
            ("AB".repeat(32), 2, "inbound", "BTC", "p", 0, 1),
            ("AB".repeat(32), 1, "outbound", "BTC", "p", 0, 0),
            ("AB".repeat(32), 1, "inbound", "LBTC", "p", 0, 0),
            ("AB".repeat(32), 1, "inbound", "BTC", "q", 0, 0),
        ] {
            let conn = cointracking_fixture(encrypted);
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('chain','ws',?,'Observed','xpub','2025-01-01')",[profile]).unwrap();
            for index in 0..copies {
                conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES(?,'ws',?,'chain',?,?,'2024-01-01T00:00:00Z',?,?,1,0,'2024-01-01')",params![format!("observed-{index}"),profile,external,format!("observed-{index}"),direction,asset]).unwrap();
            }
            let body =
                format!("{COINTRACKING_HEADER}Deposit,1,BTC,,,,,2025-01-02,{external},one\n");
            let result = import_kind(&conn, &body, Format::CoinTracking).unwrap();
            assert_eq!(
                kassiber_envelope::get(&result, "matched"),
                Some(&Py::from(matched))
            );
            assert_eq!(
                kassiber_envelope::get(&result, "ambiguous"),
                Some(&Py::from(ambiguous))
            );
            assert_eq!(
                kassiber_envelope::get(&result, "unmatched"),
                Some(&Py::from(i64::from(matched + ambiguous == 0)))
            );
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT excluded FROM transactions WHERE wallet_id='wallet'"
                ),
                i64::from(matched + ambiguous > 0)
            );
        }
    }
}

#[test]
fn python_blockpit_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_blockpit_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

const BLOCKPIT_HEADER: &str =
    "Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount,Fee Asset,Fee Amount,Comments,Trx. ID\n";
fn blockpit_fixture(encrypted: bool) -> Connection {
    let conn = fixture_codec(encrypted);
    conn.execute("UPDATE wallets SET kind='blockpit' WHERE id='wallet'", [])
        .unwrap();
    conn
}
#[test]
fn blockpit_coordinator_reconciles_identity_economics_and_idless_collisions() {
    for encrypted in [false, true] {
        let conn = blockpit_fixture(encrypted);
        conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('chain','ws','p','Observed','descriptor','{}','2025-01-01');
            INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('economic','ws','p','chain','observed','economic','2025-01-02T00:00:00Z','inbound','BTC',200000000,0,'2025-01-01');").unwrap();
        let txid = "ab".repeat(32);
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('identity','ws','p','chain',?,'identity','2025-01-01T00:00:00Z','inbound','BTC',1,0,'2025-01-01')",[&txid]).unwrap();
        let body = format!("{BLOCKPIT_HEADER}2025-01-02,Source,Deposit,,,BTC,0.002,,,one,econ\n2025-01-03,Source,Deposit,,,BTC,0.003,,,two,{txid}\n2025-01-04,Source,Deposit,,,BTC,0.004,,,three,\n2025-01-04,Source,Deposit,,,BTC,0.004,,,four,\n2025-01-05,Source,Deposit,,,BTC,0.005,,,five,active\n");
        let result = import_kind(&conn, &body, Format::Blockpit).unwrap();
        for (key, n) in [
            ("imported", 5),
            ("matched", 1),
            ("ambiguous", 3),
            ("unmatched", 1),
            ("excluded", 4),
            ("reconciliation_rows", 5),
            ("reconciliation_changed", 4),
        ] {
            assert_eq!(
                kassiber_envelope::get(&result, key),
                Some(&Py::from(n)),
                "{key}"
            );
        }
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            1
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM tags"), 2);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM tags WHERE code='blockpit-matched' AND label='Blockpit matched'"), 1);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM tags WHERE code='blockpit-ambiguous' AND label='Blockpit ambiguous'"), 1);
        assert_eq!(
            scalar(&conn, "SELECT count(*) FROM import_batch_transactions"),
            5
        );
        assert_eq!(scalar(&conn,"SELECT count(*) FROM import_batches WHERE source_format='blockpit_csv' AND rows_inserted=5"),1);
        assert_eq!(scalar(&conn,"SELECT count(DISTINCT fingerprint) FROM transactions WHERE wallet_id='wallet' AND external_id IS NULL"),2);
        let repeated = import_kind(&conn, &body, Format::Blockpit).unwrap();
        assert_eq!(
            kassiber_envelope::get(&repeated, "unchanged"),
            Some(&Py::from(5))
        );
        assert_eq!(
            kassiber_envelope::get(&repeated, "reconciliation_rows"),
            Some(&Py::from(0))
        );
        assert_eq!(
            kassiber_envelope::get(&repeated, "journal_invalidated"),
            Some(&Py::Bool(false))
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
        conn.execute("DELETE FROM transactions WHERE wallet_id='chain'", [])
            .unwrap();
        conn.execute("UPDATE transactions SET description=NULL WHERE wallet_id='wallet' AND external_id IS NOT NULL",[]).unwrap();
        let enriched = import_kind(&conn, &body, Format::Blockpit).unwrap();
        assert_eq!(
            kassiber_envelope::get(&enriched, "reconciliation_flags_cleared"),
            Some(&Py::from(2))
        );
        assert_eq!(
            kassiber_envelope::get(&enriched, "reactivated"),
            Some(&Py::from(2))
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            2
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 2);
    }
}
#[test]
fn blockpit_admission_and_late_failure_leave_every_write_uncommitted() {
    let conn = fixture();
    let error = execute(
        &conn,
        Request {
            profile_id: "p".into(),
            wallet: "Source".into(),
            file: "missing.csv".into(),
            format: Format::Blockpit,
            mode: None,
        },
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "migration_wallet_kind_required");
    assert_eq!(
        error.details,
        Some(dict([
            ("wallet", s("Source")),
            ("wallet_kind", s("custom"))
        ]))
    );
    for encrypted in [false, true] {
        let conn = blockpit_fixture(encrypted);
        let content = format!("{BLOCKPIT_HEADER}2025-01-02,Source,Deposit,,,BTC,1,,,one,\n2025-01-02,Source,Deposit,,,BTC,1,,,two,\n");
        conn.execute_batch("CREATE TRIGGER refuse_blockpit BEFORE INSERT ON import_batches BEGIN SELECT RAISE(ABORT,'denied');END;").unwrap();
        assert!(import_kind(&conn, &content, Format::Blockpit).is_err());
        for table in [
            "transactions",
            "tags",
            "transaction_tags",
            "import_batches",
            "import_batch_transactions",
        ] {
            assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
        }
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            0
        );
    }
}

#[test]
fn blockpit_authored_exclusions_survive_reconciliation_and_reactivation() {
    for encrypted in [false, true] {
        for (choice, expected) in [("true", 1), ("false", 0), ("null", 1)] {
            let conn = blockpit_fixture(encrypted);
            let body =
                format!("{BLOCKPIT_HEADER}2025-01-02,Source,Deposit,,,BTC,1,,,one,external\n");
            import_kind(&conn, &body, Format::Blockpit).unwrap();
            conn.execute("INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,wallet_id,source,changed_at,journal_input_version_after) SELECT 'edit','ws','p',id,'wallet','test','2025-01-01',1 FROM transactions",[]).unwrap();
            conn.execute("INSERT INTO transaction_edit_fields(id,event_id,field,after_value) VALUES('field','edit','excluded',?)",[choice]).unwrap();
            conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('chain','ws','p','Observed','descriptor','2025-01-01');
            INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('economic','ws','p','chain','other','economic','2025-01-02T00:00:00Z','inbound','BTC',100000000000,0,'2025-01-01');
            UPDATE transactions SET description=NULL WHERE wallet_id='wallet';").unwrap();
            let flagged = import_kind(&conn, &body, Format::Blockpit).unwrap();
            assert_eq!(
                kassiber_envelope::get(&flagged, "ambiguous"),
                Some(&Py::from(1))
            );
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT excluded FROM transactions WHERE wallet_id='wallet'"
                ),
                expected
            );
            conn.execute_batch("DELETE FROM transactions WHERE wallet_id='chain';UPDATE transactions SET description=NULL WHERE wallet_id='wallet';").unwrap();
            import_kind(&conn, &body, Format::Blockpit).unwrap();
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT excluded FROM transactions WHERE wallet_id='wallet'"
                ),
                i64::from(choice == "true")
            );
            assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 0);
        }
    }
}

#[test]
fn blockpit_promotes_ids_without_losing_occurrences_or_provider_tags() {
    for encrypted in [false, true] {
        let conn = blockpit_fixture(encrypted);
        conn.execute_batch("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('ct','ws','p','cointracking-matched','CoinTracking matched','2025-01-01');").unwrap();
        let content = format!("{BLOCKPIT_HEADER}2025-01-02,Source,Deposit,,,BTC,1,,,one,\n2025-01-02,Source,Deposit,,,BTC,1,,,two,\n");
        import_kind(&conn, &content, Format::Blockpit).unwrap();
        let ids = query_ids(&conn);
        let promoted = content.replace("one,\n", &format!("one,{}\n", "ab".repeat(32)));
        let outcome = import_kind(&conn, &promoted, Format::Blockpit).unwrap();
        assert_eq!(
            kassiber_envelope::get(&outcome, "imported"),
            Some(&Py::from(0))
        );
        assert_eq!(query_ids(&conn), ids);
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE external_id IS NOT NULL"
            ),
            1
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM tags WHERE code='cointracking-matched' AND id='ct'"
            ),
            1
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE t.code LIKE 'cointracking-%'"), 0);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE t.code='blockpit-ambiguous'"), 1);
        let repeated = import_kind(&conn, &promoted, Format::Blockpit).unwrap();
        assert_eq!(
            kassiber_envelope::get(&repeated, "journal_invalidated"),
            Some(&Py::Bool(false))
        );
    }
}

fn query_ids(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT id FROM transactions WHERE wallet_id='wallet' ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn blockpit_only_canonical_txids_establish_chain_identity() {
    for encrypted in [false, true] {
        for (external, copies, direction, asset, profile, matched, ambiguous) in [
            (
                format!("{}  {}", "ab".repeat(15), "cd".repeat(16)),
                1,
                "inbound",
                "BTC",
                "p",
                0,
                0,
            ),
            ("AB".repeat(32), 1, "inbound", "BTC", "p", 1, 0),
            ("AB".repeat(32), 2, "inbound", "BTC", "p", 0, 1),
            ("AB".repeat(32), 1, "outbound", "BTC", "p", 0, 0),
            ("AB".repeat(32), 1, "inbound", "LBTC", "p", 0, 0),
            ("AB".repeat(32), 1, "inbound", "BTC", "q", 0, 0),
        ] {
            let conn = blockpit_fixture(encrypted);
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('chain','ws',?,'Observed','xpub','2025-01-01')",[profile]).unwrap();
            for index in 0..copies {
                conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES(?,'ws',?,'chain',?,?,'2024-01-01T00:00:00Z',?,?,1,0,'2024-01-01')",params![format!("observed-{index}"),profile,external,format!("observed-{index}"),direction,asset]).unwrap();
            }
            let body =
                format!("{BLOCKPIT_HEADER}2025-01-02,Source,Deposit,,,BTC,1,,,one,{external}\n");
            let result = import_kind(&conn, &body, Format::Blockpit).unwrap();
            assert_eq!(
                kassiber_envelope::get(&result, "matched"),
                Some(&Py::from(matched))
            );
            assert_eq!(
                kassiber_envelope::get(&result, "ambiguous"),
                Some(&Py::from(ambiguous))
            );
            assert_eq!(
                kassiber_envelope::get(&result, "unmatched"),
                Some(&Py::from(i64::from(matched + ambiguous == 0)))
            );
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT excluded FROM transactions WHERE wallet_id='wallet'"
                ),
                i64::from(matched + ambiguous > 0)
            );
        }
    }
}

#[test]
fn python_binance_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_binance_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

const BINANCE_AUTO: &str = "timestamp UTC,base asset symbol,quote asset amount + symbol,trading fee (in quote asset),base asset amount + symbol,source of funds\n2025-01-02 03:04:05,BTC,100.00 EUR,1.00 EUR,0.002 BTC,Spot Wallet\n";
const BINANCE_REF: &str = "binance-supplemental:2025-01-02 03:04:05:autoinvest:BTC:0.002:1";
fn binance_import(
    conn: &Connection,
    content: &str,
    mode: &str,
    wallet: &str,
) -> Result<Py, AppError> {
    let file = std::env::temp_dir().join(format!(
        "binance-coordinator-{}-{:?}.csv",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&file, content).unwrap();
    let result = execute(
        conn,
        Request {
            profile_id: "p".into(),
            wallet: wallet.into(),
            file: file.to_str().unwrap().into(),
            format: Format::Binance,
            mode: Some(mode.into()),
        },
        None,
    );
    std::fs::remove_file(file).unwrap();
    result
}
#[test]
fn binance_full_coordinator_preserves_overlap_reference_ids_enrichment_and_reimports() {
    for encrypted in [false, true] {
        let conn = fixture_codec(encrypted);
        conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
            .unwrap();
        conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('chain','ws','p','Observed','descriptor','{}','2025-01-01');").unwrap();
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('observed','ws','p','chain',?,'observed','2025-01-01T00:00:00Z','inbound','BTC',200000000,1,'2025-01-01')", [BINANCE_REF]).unwrap();
        let first = binance_import(&conn, BINANCE_AUTO, "full", "Source").unwrap();
        for (key, value) in [
            ("imported", 1),
            ("matched", 1),
            ("unmatched", 0),
            ("ambiguous", 0),
        ] {
            assert_eq!(
                kassiber_envelope::get(&first, key),
                Some(&Py::from(value)),
                "{key}"
            );
        }
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE wallet_id='wallet' AND excluded=1 AND external_id_kind IS NULL"),1);
        assert_eq!(scalar(&conn,"SELECT count(*) FROM tags WHERE code='binance-matched' AND label='Binance matched'"),1);
        assert_eq!(scalar(&conn,"SELECT count(*) FROM tags WHERE code='binance-wallet-gap' AND label='Binance wallet gap'"),1);
        assert_eq!(scalar(&conn,"SELECT count(*) FROM tags WHERE code='binance-ambiguous' AND label='Binance ambiguous'"),1);
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE wallet_id='chain' AND fiat_value_exact IS NULL AND fee=1"),1);
        let repeated = binance_import(&conn, BINANCE_AUTO, "full", "Source").unwrap();
        assert_eq!(
            kassiber_envelope::get(&repeated, "imported"),
            Some(&Py::from(0))
        );
        assert_eq!(scalar(&conn,"SELECT count(*) FROM import_batches WHERE source_format='binance_supplemental_csv'"),1);
        let enriched = binance_import(
            &conn,
            &BINANCE_AUTO.replace("100.00 EUR", "200.00 EUR"),
            "full",
            "Source",
        )
        .unwrap();
        assert_eq!(
            kassiber_envelope::get(&enriched, "imported"),
            Some(&Py::from(0))
        );
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE wallet_id='wallet' AND fiat_value_exact='201.00' AND fiat_rate_exact='100500'"),1);
        conn.execute(
            "UPDATE transactions SET external_id_kind='txid' WHERE wallet_id='chain'",
            [],
        )
        .unwrap();
        binance_import(&conn, BINANCE_AUTO, "full", "Source").unwrap();
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE wallet_id='wallet' AND external_id_kind IS NULL"),1);
        conn.execute("DELETE FROM transactions WHERE wallet_id='chain'", [])
            .unwrap();
        let unmatched = binance_import(&conn, BINANCE_AUTO, "full", "Source").unwrap();
        assert_eq!(
            kassiber_envelope::get(&unmatched, "unmatched"),
            Some(&Py::from(1))
        );
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='binance-wallet-gap'"),1);
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='binance-matched'"),0);
        let scaled = BINANCE_AUTO.replace("0.002 BTC", "0.0020 BTC");
        assert_eq!(
            kassiber_envelope::get(
                &binance_import(&conn, &scaled, "full", "Source").unwrap(),
                "imported"
            ),
            Some(&Py::from(1))
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE external_id_kind='txid'"
            ),
            0
        );
        let dividend = format!(
            "id,amount,asset,divTime\n{},0.00001,BTC,1735776000123\n",
            "ab".repeat(32)
        );
        binance_import(&conn, &dividend, "full", "Source").unwrap();
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE external_id LIKE 'binance:%' AND external_id_kind IS NULL"),1);
    }
}
#[test]
fn binance_relevant_coordinator_matches_scope_fee_precedence_and_price_refusals() {
    for encrypted in [false, true] {
        let conn = fixture_codec(encrypted);
        assert_eq!(
            kassiber_envelope::get(
                &binance_import(&conn, BINANCE_AUTO, "relevant", "").unwrap(),
                "skipped_unmatched"
            ),
            Some(&Py::from(1))
        );
        conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
            .unwrap();
        import(
            &conn,
            &format!("date,txid,amount,fee\n2025-01-01,{BINANCE_REF},0.002,0.00001\n"),
        )
        .unwrap();
        let result = binance_import(&conn, BINANCE_AUTO, "relevant", "Source").unwrap();
        assert_eq!(
            kassiber_envelope::get(&result, "matched"),
            Some(&Py::from(1))
        );
        assert_eq!(
            kassiber_envelope::get(&result, "binance_rows"),
            Some(&Py::from(1))
        );
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE fee=1000000 AND fiat_value_exact='101.00' AND external_id_kind IS NULL"),1);
        let Py::Dict(entries) = &result else {
            panic!("enrichment response must be a dict")
        };
        let position = |key: &str| {
            entries
                .iter()
                .position(|(k, _)| *k == kassiber_pyjson::Key::from(key))
                .unwrap()
        };
        assert!(position("updated_records") < position("updated"));
        let version = scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'",
        );
        let repeated = binance_import(&conn, BINANCE_AUTO, "relevant", "Source").unwrap();
        assert_eq!(
            kassiber_envelope::get(&repeated, "journal_invalidated"),
            Some(&Py::Bool(true))
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            version + 1
        );
        conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('other','ws','p','Other','custom','{}','2025-01-01');
        INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) SELECT 'copy',workspace_id,profile_id,'other',external_id,'copy',occurred_at,direction,asset,amount,fee,created_at FROM transactions LIMIT 1;").unwrap();
        assert_eq!(
            kassiber_envelope::get(
                &binance_import(&conn, BINANCE_AUTO, "relevant", "").unwrap(),
                "skipped_ambiguous"
            ),
            Some(&Py::from(1))
        );
        conn.execute("UPDATE transactions SET fee=0 WHERE id='copy'", [])
            .unwrap();
        assert_eq!(
            kassiber_envelope::get(
                &binance_import(&conn, BINANCE_AUTO, "relevant", "").unwrap(),
                "matched"
            ),
            Some(&Py::from(1))
        );
        conn.execute("UPDATE transactions SET profile_id='q' WHERE id='copy'", [])
            .unwrap();
        conn.execute("UPDATE profiles SET fiat_currency='USD' WHERE id='p'", [])
            .unwrap();
        let error = binance_import(&conn, BINANCE_AUTO, "relevant", "").unwrap_err();
        assert_eq!(error.code, "validation");
        assert!(error.message.contains("currency EUR does not match"));
    }
}
#[test]
fn binance_late_failure_rolls_back_records_flags_runs_and_readiness() {
    for encrypted in [false, true] {
        for value in ["NaN123", "-NaN42", "NaN１２３", "+NaN0001"] {
            let conn = fixture_codec(encrypted);
            let body = format!("id,amount,asset,divTime\nreward,0.00001,BTC,{value}\n");
            let error = binance_import(&conn, &body, "full", "Source").unwrap_err();
            assert_eq!(error.message, "Invalid value NaN (not a number)");
            assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
        }
        for mode in ["full", "relevant"] {
            let conn = fixture_codec(encrypted);
            conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
                .unwrap();
            if mode == "full" {
                conn.execute_batch("CREATE TRIGGER refuse_binance BEFORE INSERT ON import_batches BEGIN SELECT RAISE(ABORT,'denied');END;").unwrap();
            } else {
                import(
                    &conn,
                    &format!("date,txid,amount\n2025-01-01,{BINANCE_REF},0.002\n"),
                )
                .unwrap();
                conn.execute_batch("CREATE TRIGGER refuse_binance BEFORE UPDATE OF journal_input_version ON profiles BEGIN SELECT RAISE(ABORT,'denied');END;").unwrap();
            }
            let before = scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'",
            );
            let count = scalar(&conn, "SELECT count(*) FROM transactions");
            let runs = scalar(&conn, "SELECT count(*) FROM import_batches");
            assert!(binance_import(&conn, BINANCE_AUTO, mode, "Source").is_err());
            assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), count);
            assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), runs);
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT count(*) FROM transactions WHERE fiat_value_exact IS NOT NULL"
                ),
                0
            );
            assert_eq!(scalar(&conn, "SELECT count(*) FROM tags"), 0);
            assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 0);
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT journal_input_version FROM profiles WHERE id='p'"
                ),
                before
            );
        }
    }
}
#[test]
fn binance_default_wallet_and_admission_follow_reference_order() {
    let conn = fixture();
    conn.execute_batch("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','2025-01-01');").unwrap();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    binance_import(&conn, BINANCE_AUTO, "full", "").unwrap();
    assert_eq!(scalar(&conn,"SELECT count(*) FROM wallets WHERE label='Binance' AND kind='binance' AND config_json='{\"source_format\": \"binance_supplemental_csv\"}'"),1);
    assert_eq!(
        kassiber_envelope::get(
            &binance_import(&conn, BINANCE_AUTO, "full", "").unwrap(),
            "imported"
        ),
        Some(&Py::from(0))
    );
    assert_eq!(
        binance_import(&conn, BINANCE_AUTO, "bad", "missing")
            .unwrap_err()
            .code,
        "validation"
    );
    assert_eq!(
        binance_import(&conn, BINANCE_AUTO, "full", "missing")
            .unwrap_err()
            .code,
        "not_found"
    );
}

#[test]
fn python_btcpay_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_btcpay_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

#[test]
fn python_ledger_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_ledger_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

#[test]
fn python_ledger_preview_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_ledger_imports.json"
    ))
    .unwrap();
    for (i, case) in cases.as_array().unwrap().iter().enumerate() {
        let path = std::env::temp_dir().join(format!(
            "ledger-preview-{}-{i}{}",
            std::process::id(),
            case["extension"].as_str().unwrap()
        ));
        let bytes: Vec<u8> = case
            .get("bytes")
            .map(|v| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|b| b.as_u64().unwrap() as u8)
                    .collect()
            })
            .unwrap_or_else(|| case["text"].as_str().unwrap().as_bytes().to_vec());
        std::fs::write(&path, bytes).unwrap();
        let mapping = case.get("column_map").filter(|v| !v.is_null()).map(|_| {
            ledger::ColumnMap::from_python(
                &kassiber_pytext::loads(case["column_map_json"].as_str().unwrap()).unwrap(),
            )
            .unwrap()
        });
        let result = ledger::preview(
            path.to_str().unwrap(),
            mapping.as_ref(),
            case["limit"].as_i64().unwrap().max(0) as usize,
        );
        std::fs::remove_file(&path).unwrap();
        match result {
            Ok(value) => assert_eq!(
                serde_json::from_str::<Json>(
                    &kassiber_pyjson::dumps(&value, &Default::default()).unwrap()
                )
                .unwrap(),
                case["preview"],
                "{}",
                case["name"]
            ),
            Err(e) => {
                let details = e
                    .details
                    .as_ref()
                    .map(|v| kassiber_pyjson::dumps(v, &Default::default()).unwrap())
                    .unwrap_or_else(|| "null".into());
                assert_eq!(
                    json!({"code":e.code,"message":e.message.replace(path.to_str().unwrap(),"{file}"),"hint":e.hint,"details":serde_json::from_str::<Json>(&details).unwrap(),"details_json":details}),
                    case["preview_error"],
                    "{}",
                    case["name"]
                );
            }
        }
    }
}

#[test]
fn ledger_coordinator_keeps_supplied_maps_scope_dedupe_and_atomic_batches() {
    for encrypted in [false, true] {
        let conn = fixture_codec(encrypted);
        conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
            .unwrap();
        let path = std::env::temp_dir().join(format!(
            "ledger-coordinator-{}-{encrypted}.csv",
            std::process::id()
        ));
        std::fs::write(&path,"When,Units,Side,Price,Currency,Hash,Funding,Output\n2025-01-02,0.002,ACQUIRE,190,EUR,abababababababababababababababababababababababababababababababab,cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd,2\n").unwrap();
        let mapping = ledger::ColumnMap::from_json(
            &json!({"date":"When","amount":"Units","type":"Side","fiat_value":"Price","fiat_currency":"Currency","type_map":{"ACQUIRE":"Buy"},"payment_hash":"Hash","swap_refund_funding_txid":"Funding","swap_refund_funding_vout":"Output"}),
        );
        let run = |wallet: &str| {
            execute_with_ledger_map(
                &conn,
                Request {
                    profile_id: "p".into(),
                    wallet: wallet.into(),
                    file: path.to_string_lossy().into_owned(),
                    format: Format::Ledger,
                    mode: None,
                },
                Some(&mapping),
                None,
            )
        };
        let first = run("Source").unwrap();
        assert_eq!(
            kassiber_envelope::get(&first, "imported"),
            Some(&Py::from(1))
        );
        assert_eq!(scalar(&conn, "SELECT amount FROM transactions"), 200000000);
        assert_eq!(
            scalar(&conn, "SELECT swap_refund_funding_vout FROM transactions"),
            2
        );
        let stored: (String, String, String) = conn
            .query_row(
                "SELECT source_format,column_map_json,source_filename FROM import_batches",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            stored,
            (
                "generic_ledger".into(),
                config::stored_json(&mapping.raw),
                path.file_name().unwrap().to_string_lossy().into_owned()
            )
        );
        let hash: String = conn
            .query_row("SELECT payment_hash_source FROM transactions", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(hash, "generic_ledger");
        let second = run("Source").unwrap();
        assert_eq!(
            kassiber_envelope::get(&second, "imported"),
            Some(&Py::from(0))
        );
        assert_eq!(
            kassiber_envelope::get(&second, "journal_invalidated"),
            Some(&Py::Bool(true))
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
        assert_eq!(run("missing").unwrap_err().code, "not_found");
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('other','ws','p','Other','custom','2025-01-01')",[]).unwrap();
        assert_eq!(
            kassiber_envelope::get(&run("Other").unwrap(), "imported"),
            Some(&Py::from(1))
        );
        let before = scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'",
        );
        conn.execute_batch("DELETE FROM import_batch_transactions;DELETE FROM import_batches;DELETE FROM transactions;CREATE TRIGGER deny_ledger_link BEFORE INSERT ON import_batch_transactions BEGIN SELECT RAISE(ABORT,'ledger link denied');END;").unwrap();
        let error = run("Source").unwrap_err();
        assert_eq!(error.code, "internal_error");
        assert_eq!(
            kassiber_envelope::get(error.details.as_ref().unwrap(), "sqlite_error_code"),
            Some(&Py::from(1811))
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 0);
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            before
        );
        assert!(conn.is_autocommit());
        conn.execute_batch("DROP TRIGGER deny_ledger_link").unwrap();
        assert_eq!(
            kassiber_envelope::get(&run("Source").unwrap(), "imported"),
            Some(&Py::from(1))
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn btcpay_coordinator_preserves_notes_tags_fee_inclusion_and_reimports() {
    for encrypted in [false, true] {
        let conn = fixture_codec(encrypted);
        let body = json!([{"TransactionId":"ab".repeat(32),"Timestamp":"2025-01-02","Amount":"-0.002 BTC","Comment":" imported ","Labels":["Two Words","two-words","Other"]}]).to_string();
        let first = import_kind(&conn, &body, Format::BtcpayJson).unwrap();
        assert_eq!(scalar(&conn, "SELECT amount FROM transactions"), 200000000);
        assert_eq!(scalar(&conn, "SELECT fee FROM transactions"), 0);
        assert_eq!(
            scalar(&conn, "SELECT amount_includes_fee FROM transactions"),
            1
        );
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE direction='outbound' AND kind='withdrawal' AND external_id_kind='txid' AND note='imported'"),1);
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM tags WHERE code='two-words' AND label='Two Words'"
            ),
            1
        );
        assert_eq!(
            kassiber_envelope::get(&first, "btcpay_notes_set"),
            Some(&Py::from(1))
        );
        assert_eq!(
            kassiber_envelope::get(&first, "btcpay_tags_created"),
            Some(&Py::from(2))
        );
        assert_eq!(
            kassiber_envelope::get(&first, "btcpay_tags_added"),
            Some(&Py::from(2))
        );
        let repeated = import_kind(&conn, &body, Format::BtcpayJson).unwrap();
        assert_eq!(
            kassiber_envelope::get(&repeated, "unchanged"),
            Some(&Py::from(1))
        );
        assert_eq!(
            kassiber_envelope::get(&repeated, "journal_invalidated"),
            Some(&Py::Bool(false))
        );
        assert_eq!(
            kassiber_envelope::get(&repeated, "btcpay_tags_added"),
            Some(&Py::from(0))
        );
        conn.execute_batch("UPDATE transactions SET note='authored';DELETE FROM transaction_tags;DELETE FROM tags;").unwrap();
        let tagged = import_kind(&conn, &body, Format::BtcpayJson).unwrap();
        assert_eq!(
            kassiber_envelope::get(&tagged, "journal_invalidated"),
            Some(&Py::Bool(true))
        );
        assert_eq!(
            kassiber_envelope::get(&tagged, "btcpay_notes_set"),
            Some(&Py::from(0))
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE note='authored'"
            ),
            1
        );
        conn.execute_batch("UPDATE transactions SET note=NULL;")
            .unwrap();
        let noted = import_kind(&conn, &body, Format::BtcpayJson).unwrap();
        assert_eq!(
            kassiber_envelope::get(&noted, "btcpay_notes_set"),
            Some(&Py::from(1))
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            3
        );
        assert_eq!(scalar(&conn,"SELECT count(*) FROM import_batches WHERE source_format='btcpay_json' AND rows_inserted=1"),1);
        let idless = json!([{"Timestamp":"2025-01-03","Amount":"0.001 BTC","Comment":"unlinked","Labels":"unlinked"}]).to_string();
        let unlinked = import_kind(&conn, &idless, Format::BtcpayJson).unwrap();
        assert_eq!(
            kassiber_envelope::get(&unlinked, "btcpay_notes_set"),
            Some(&Py::from(0))
        );
        assert_eq!(
            scalar(&conn, "SELECT count(*) FROM tags WHERE code='unlinked'"),
            0
        );
    }
}

#[test]
fn btcpay_late_failure_rolls_back_transactions_metadata_runs_and_readiness() {
    for encrypted in [false, true] {
        for trigger in [
            "CREATE TRIGGER refuse_btcpay BEFORE INSERT ON import_batches BEGIN SELECT RAISE(ABORT,'denied');END;",
            "CREATE TRIGGER refuse_btcpay BEFORE INSERT ON tags BEGIN SELECT RAISE(ABORT,'denied');END;",
            "CREATE TRIGGER refuse_btcpay BEFORE INSERT ON transaction_tags BEGIN SELECT RAISE(ABORT,'denied');END;",
            "CREATE TRIGGER refuse_btcpay BEFORE UPDATE OF journal_input_version ON profiles BEGIN SELECT RAISE(ABORT,'denied');END;",
        ] {
            let conn = fixture_codec(encrypted);
            conn.execute_batch(trigger).unwrap();
            let body = "TransactionId,Timestamp,Amount,Comment,Labels\nref,2025-01-02,1 BTC,note,label\n";
            assert!(import_kind(&conn,body,Format::BtcpayCsv).is_err());
            for table in ["transactions","tags","transaction_tags","import_batches","import_batch_transactions"] {
                assert_eq!(scalar(&conn,&format!("SELECT count(*) FROM {table}")),0,"{table}");
            }
            assert_eq!(scalar(&conn,"SELECT journal_input_version FROM profiles WHERE id='p'"),0);
        }
        let conn = fixture_codec(encrypted);
        let metadata_body =
            "TransactionId,Timestamp,Amount,Comment,Labels\nref,2025-01-02,1 BTC,note,label\n";
        import_kind(&conn, metadata_body, Format::BtcpayCsv).unwrap();
        conn.execute_batch("UPDATE transactions SET note=NULL;DELETE FROM transaction_tags;DELETE FROM tags;
            CREATE TRIGGER refuse_btcpay BEFORE UPDATE OF journal_input_version ON profiles BEGIN SELECT RAISE(ABORT,'denied');END;").unwrap();
        assert!(import_kind(&conn, metadata_body, Format::BtcpayCsv).is_err());
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE note IS NULL"
            ),
            1
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM tags"), 0);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 0);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            1
        );
        let conn = fixture_codec(encrypted);
        let body = "TransactionId,Timestamp,Amount\nfirst,2025-01-02,1 BTC\nsecond,invalid,1 BTC\n";
        assert!(import_kind(&conn, body, Format::BtcpayCsv).is_err());
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
    }
}

#[test]
fn btcpay_metadata_lookup_retains_untrimmed_typed_and_scoped_reference_behavior() {
    for encrypted in [false, true] {
        let conn = fixture_codec(encrypted);
        conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('elsewhere','ws','q','Elsewhere','custom','2025-01-01'),('same-profile','ws','p','Other wallet','custom','2025-01-01');
            INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('elsewhere','ws','q','elsewhere','clean','elsewhere','2025-01-02T00:00:00Z','inbound','BTC',100000000000,0,'2025-01-01'),('same-profile','ws','p','same-profile','clean','same-profile','2025-01-02T00:00:00Z','inbound','BTC',100000000000,0,'2025-01-01');").unwrap();
        let body = json!([
            {"TransactionId":" padded ","Timestamp":"2025-01-02","Amount":"1 BTC","Comment":"padded","Labels":"padded"},
            {"TransactionId":true,"Timestamp":"2025-01-02","Amount":"1 BTC","Comment":"boolean","Labels":"boolean"},
            {"TransactionId":1,"Timestamp":"2025-01-02","Amount":"1 BTC","Comment":"integer","Labels":"integer"},
            {"TransactionId":"clean","Timestamp":"2025-01-02","Amount":"1 BTC","Comment":"clean","Labels":"clean"}
        ]).to_string();
        let result = import_kind(&conn, &body, Format::BtcpayJson).unwrap();
        assert_eq!(
            kassiber_envelope::get(&result, "btcpay_notes_set"),
            Some(&Py::from(3))
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE external_id=' padded ' AND note='padded'"
            ),
            1
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE external_id='True' AND note IS NULL"
            ),
            1
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE wallet_id IN ('elsewhere','same-profile') AND note IS NULL"
            ),
            2
        );
        assert_eq!(
            scalar(&conn, "SELECT count(*) FROM tags WHERE code='boolean'"),
            1
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE external_id='1' AND note='boolean'"
            ),
            1
        );
        let body = json!([{"TransactionId":["invalid"],"Timestamp":"2025-01-02","Amount":"1 BTC","Comment":"note"}]).to_string();
        let error = import_kind(&conn, &body, Format::BtcpayJson).unwrap_err();
        assert_eq!(
            error.message,
            if encrypted {
                "Error binding parameter 2 - probably unsupported type."
            } else {
                "Error binding parameter 3: type 'list' is not supported"
            }
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 6);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
    }
}

#[test]
fn btcpay_labels_preserve_provider_shapes_normalization_and_idempotence() {
    for encrypted in [false, true] {
        for (labels, expected) in [
            (
                json!({"fallback":{},"empty":null,"text":{"text":" Trimmed "},"skip":{"text":""},"boolean":false,"zero":0}),
                vec![
                    ("fallback", "fallback"),
                    ("empty", "empty"),
                    ("trimmed", "Trimmed"),
                    ("false", "False"),
                    ("0", "0"),
                ],
            ),
            (
                json!([{"text":" Nested "},{},null,0,false," "]),
                vec![("nested", "Nested"), ("0", "0"), ("false", "False")],
            ),
            (json!(" A, A , B "), vec![("a", "A"), ("b", "B")]),
        ] {
            let conn = fixture_codec(encrypted);
            let body = json!([{"TransactionId":"labels","Timestamp":"2025-01-02","Amount":"1 BTC","Labels":labels}]).to_string();
            let result = import_kind(&conn, &body, Format::BtcpayJson).unwrap();
            assert_eq!(
                kassiber_envelope::get(&result, "btcpay_tags_added"),
                Some(&Py::from(expected.len() as i64))
            );
            let mut statement = conn
                .prepare("SELECT code,label FROM tags ORDER BY code")
                .unwrap();
            let observed = statement
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let mut expected = expected
                .into_iter()
                .map(|(code, label)| (code.to_string(), label.to_string()))
                .collect::<Vec<_>>();
            expected.sort();
            assert_eq!(observed, expected);
            let repeated = import_kind(&conn, &body, Format::BtcpayJson).unwrap();
            assert_eq!(
                kassiber_envelope::get(&repeated, "btcpay_tags_added"),
                Some(&Py::from(0))
            );
            assert_eq!(
                kassiber_envelope::get(&repeated, "journal_invalidated"),
                Some(&Py::Bool(false))
            );
        }
    }
}

#[test]
fn btcpay_surrogate_labels_refuse_without_erasing_evidence_or_partial_writes() {
    for encrypted in [false, true] {
        for labels in [r#"" İ\ud800 ""#, r#"[" İ\ud800 "]"#, r#"{" İ\ud800 ":{}}"#] {
            let conn = fixture_codec(encrypted);
            let body = format!(
                r#"[{{"TransactionId":"labels","Timestamp":"2025-01-02","Amount":"1 BTC","Labels":{labels}}}]"#
            );
            let error = import_kind(&conn, &body, Format::BtcpayJson).unwrap_err();
            assert_eq!(error.message,"'utf-8' codec can't encode character '\\ud800' in position 2: surrogates not allowed");
            for table in ["transactions", "tags", "transaction_tags", "import_batches"] {
                assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
            }
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT journal_input_version FROM profiles WHERE id='p'"
                ),
                0
            );
        }
    }
}

#[test]
fn btcpay_surrogate_currency_preserves_refusal_precedence_and_rollback() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_btcpay_imports.json"
    ))
    .unwrap();
    let cases = cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| {
            case["name"]
                .as_str()
                .unwrap()
                .starts_with("btcpay:json:currency-surrogate-")
        })
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), 6);
    for encrypted in [false, true] {
        for case in &cases {
            let conn = fixture_codec(encrypted);
            let error =
                import_kind(&conn, case["text"].as_str().unwrap(), Format::BtcpayJson).unwrap_err();
            assert_eq!(
                json!({"code":error.code,"message":error.message,"hint":error.hint}),
                case["error"],
                "{}",
                case["name"]
            );
            for table in ["transactions", "tags", "transaction_tags", "import_batches"] {
                assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
            }
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT journal_input_version FROM profiles WHERE id='p'"
                ),
                0
            );
        }
    }
}

#[test]
fn python_wasabi_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_wasabi_imports.json"
    ))
    .unwrap();
    assert_import_vectors(&cases);
}

#[test]
fn wasabi_coordinator_preserves_review_notes_inventory_and_identities() {
    for encrypted in [false, true] {
        let conn = fixture_codec(encrypted);
        let result = import_kind(&conn, WASABI_HISTORY, Format::Wasabi).unwrap();
        assert!(kassiber_envelope::get(&result, "import_batch_id").is_none());
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 0);
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE kind='coinjoin' AND review_status='review' AND note='receipt' AND amount=12345000"),1);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 3);
        assert_eq!(scalar(&conn,"SELECT count(*) FROM wallet_utxos WHERE amount=12345000 AND branch_index=1 AND address_index=5 AND anonymity_score=12 AND spent_at IS NULL"),1);
        let config: String = conn
            .query_row("SELECT config_json FROM wallets", [], |r| r.get(0))
            .unwrap();
        assert!(config.contains("84'/0'/*'"));
        assert!(!config.contains("secret"));
        let config: Json = serde_json::from_str(&config).unwrap();
        assert_eq!(
            config["wasabi_metadata"]["segwitAccountPathHint"],
            "84'/0'/*'"
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            1
        );
        let repeated = import_kind(&conn, WASABI_HISTORY, Format::Wasabi).unwrap();
        assert_eq!(
            kassiber_envelope::get(&repeated, "journal_invalidated"),
            Some(&Py::Bool(false))
        );
        conn.execute_batch("UPDATE transactions SET note='authored',review_status='completed'")
            .unwrap();
        import_kind(&conn, WASABI_HISTORY, Format::Wasabi).unwrap();
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE note='authored' AND review_status='completed'"),1);
        conn.execute_batch("UPDATE transactions SET review_status='review'")
            .unwrap();
        let plain = WASABI_HISTORY.replace("true", "false");
        conn.execute_batch("UPDATE transactions SET privacy_boundary='payjoin'")
            .unwrap();
        import_kind(&conn, &plain, Format::Wasabi).unwrap();
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE privacy_boundary='payjoin' AND review_status='review'"),1);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 3);
        conn.execute_batch("UPDATE transactions SET privacy_boundary=NULL")
            .unwrap();
        let metadata_only = import_kind(&conn, &plain, Format::Wasabi).unwrap();
        assert_eq!(
            kassiber_envelope::get(&metadata_only, "journal_invalidated"),
            Some(&Py::Bool(true))
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE review_status IS NULL AND kind='deposit'"
            ),
            1
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transaction_tags"), 1);
        import_kind(&conn, "{\"listcoins\":[]}", Format::Wasabi).unwrap();
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM wallet_utxos WHERE spent_at IS NOT NULL"
            ),
            1
        );
        assert_eq!(
            scalar(&conn, "SELECT observed_count FROM wallet_utxo_refreshes"),
            0
        );
        import_kind(&conn, "{}", Format::Wasabi).unwrap();
        assert_eq!(
            scalar(&conn, "SELECT observed_count FROM wallet_utxo_refreshes"),
            0
        );
    }
}

#[test]
fn wasabi_late_failure_rolls_back_transactions_metadata_inventory_and_readiness() {
    for encrypted in [false, true] {
        for sql in ["CREATE TRIGGER refuse_wasabi BEFORE INSERT ON tags BEGIN SELECT RAISE(ABORT,'denied');END;", "CREATE TRIGGER refuse_wasabi BEFORE UPDATE OF config_json ON wallets BEGIN SELECT RAISE(ABORT,'denied');END;", "CREATE TRIGGER refuse_wasabi BEFORE INSERT ON wallet_utxo_refreshes BEGIN SELECT RAISE(ABORT,'denied');END;", ""] {
            let conn=fixture_codec(encrypted);conn.execute_batch(sql).unwrap();
            let bad=if sql.is_empty() {WASABI_HISTORY.replace("\"vout\":0","\"vout\":-1")} else {WASABI_HISTORY.into()};
            assert!(import_kind(&conn,&bad,Format::Wasabi).is_err());
            for table in ["transactions","tags","transaction_tags","wallet_utxos","wallet_utxo_refreshes","import_batches"] {assert_eq!(scalar(&conn,&format!("SELECT count(*) FROM {table}")),0,"{table}");}
            assert_eq!(scalar(&conn,"SELECT journal_input_version FROM profiles WHERE id='p'"),0);
            let config:String=conn.query_row("SELECT config_json FROM wallets",[],|r|r.get(0)).unwrap();assert_eq!(config,"{}");
        }
    }
}

#[test]
fn wasabi_bundle_metadata_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_wasabi_imports.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let Some(expected) = case.get("bundle") else {
            continue;
        };
        let file = std::env::temp_dir().join(format!("wasabi-bundle-{}.json", std::process::id()));
        std::fs::write(&file, case["text"].as_str().unwrap()).unwrap();
        let bundle = wasabi::load(file.to_str().unwrap()).unwrap();
        std::fs::remove_file(file).unwrap();
        let actual = dict([
            ("coins", Py::List(bundle.coins)),
            ("metadata", bundle.metadata),
            ("coin_sections_present", Py::Bool(bundle.coin_sections)),
            ("wallet_json_present", Py::Bool(bundle.wallet_json)),
            ("listkeys_count", Py::from(bundle.keys as i64)),
            ("payments_in_coinjoin", Py::List(bundle.payment_rows)),
        ]);
        assert_eq!(
            kassiber_pytext::loads(&wasabi::dump(&actual).unwrap()).unwrap(),
            kassiber_pytext::loads(&expected.to_string()).unwrap(),
            "{}",
            case["name"]
        );
    }
    let file = std::env::temp_dir().join(format!("wasabi-surrogate-{}.json", std::process::id()));
    std::fs::write(
        &file,
        r#"{"listkeys":[{"keyState":"\ud800"},{"keyState":"\ud800"},{"keyState":""}]}"#,
    )
    .unwrap();
    let bundle = wasabi::load(file.to_str().unwrap()).unwrap();
    std::fs::remove_file(file).unwrap();
    assert_eq!(
        bundle.metadata,
        kassiber_pytext::loads(r#"{"keyStateCounts":{"\ud800":2}}"#).unwrap()
    );
}

#[test]
fn wasabi_surrogate_inputs_cannot_become_valid_quantities_or_flags() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_wasabi_codepoints.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let file =
            std::env::temp_dir().join(format!("wasabi-codepoints-{}.json", std::process::id()));
        std::fs::write(&file, case["text"].as_str().unwrap()).unwrap();
        let result = wasabi::load(file.to_str().unwrap());
        std::fs::remove_file(file).unwrap();
        let (actual, expected) = if let Some(expected) = case.get("error_json") {
            let error = result.err().expect("invalid code points must be refused");
            let envelope = error.envelope(None);
            let fields = kassiber_envelope::get(&envelope, "error").unwrap();
            (
                dict(
                    ["code", "message", "hint"]
                        .map(|key| (key, kassiber_envelope::get(fields, key).unwrap().clone())),
                ),
                expected.as_str().unwrap(),
            )
        } else {
            let bundle = result.unwrap();
            (
                dict([
                    ("records", Py::List(bundle.records)),
                    ("coins", Py::List(bundle.coins)),
                    ("metadata", bundle.metadata),
                    ("coin_sections_present", Py::Bool(bundle.coin_sections)),
                    ("wallet_json_present", Py::Bool(bundle.wallet_json)),
                    ("listkeys_count", Py::from(bundle.keys as i64)),
                    ("payments_in_coinjoin", Py::List(bundle.payment_rows)),
                ]),
                case["bundle_json"].as_str().unwrap(),
            )
        };
        assert_eq!(wasabi::dump(&actual).unwrap(), expected, "{}", case["name"]);
        if let Some(expected) = case.get("coordinator_error_json") {
            for encrypted in [false, true] {
                let conn = fixture_codec(encrypted);
                let error = import_kind(&conn, case["text"].as_str().unwrap(), Format::Wasabi)
                    .expect_err("reference refusal must leave no writes");
                let envelope = error.envelope(None);
                let fields = kassiber_envelope::get(&envelope, "error").unwrap();
                let actual = dict(
                    ["code", "message", "hint"]
                        .map(|key| (key, kassiber_envelope::get(fields, key).unwrap().clone())),
                );
                assert_eq!(
                    wasabi::dump(&actual).unwrap(),
                    expected.as_str().unwrap(),
                    "{}",
                    case["name"]
                );
                for table in [
                    "transactions",
                    "tags",
                    "transaction_tags",
                    "wallet_utxos",
                    "wallet_utxo_refreshes",
                    "import_batches",
                ] {
                    assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
                }
                assert_eq!(
                    scalar(
                        &conn,
                        "SELECT journal_input_version FROM profiles WHERE id='p'"
                    ),
                    0
                );
            }
        }
    }
}

#[test]
fn wasabi_missing_file_refusal_is_atomic() {
    let file = std::env::temp_dir().join(format!("wasabi-missing-{}.json", std::process::id()));
    assert!(!file.exists());
    for encrypted in [false, true] {
        let conn = fixture_codec(encrypted);
        let error = execute(
            &conn,
            Request {
                profile_id: "p".into(),
                wallet: "Source".into(),
                file: file.to_string_lossy().into_owned(),
                format: Format::Wasabi,
                mode: None,
            },
            None,
        )
        .unwrap_err();
        assert_eq!(error.code, "internal_error");
        assert_eq!(
            error.message,
            format!(
                "[Errno 2] No such file or directory: {}",
                kassiber_pytext::repr_str(file.to_str().unwrap())
            )
        );
        assert!(error.hint.is_none());
        for table in [
            "transactions",
            "tags",
            "transaction_tags",
            "wallet_utxos",
            "wallet_utxo_refreshes",
            "import_batches",
        ] {
            assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
        }
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            0
        );
        assert!(conn.is_autocommit());
    }
}

const WASABI_HISTORY: &str = r#"{"gethistory":[{"tx":"abababababababababababababababababababababababababababababababab","datetime":"2025-01-02","amount":12345,"label":"receipt","islikelycoinjoin":true}],"listcoins":[{"txid":"abababababababababababababababababababababababababababababababab","vout":0,"amount":12345,"keyPath":"m/84'/0'/0'/1/5","anonymityScore":12}],"wallet_json":{"EncryptedSecret":"secret","SegWitAccountKeyPath":"m/84'/0'/7'"}}"#;

/// tests/rust_port/repro_import_json_format_on_csv.py: a file under a JSON
/// format that is not UTF-8 JSON is a wrong-format choice.
#[test]
fn json_formats_refuse_a_non_json_file_as_validation() {
    let dir = std::env::temp_dir().join(format!(
        "kassiber-json-refusal-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    };
    let csv = write(
        "export.csv",
        b"date,txid,direction,asset,amount\n2025-01-02,ab,inbound,BTC,0.1\n",
    );
    let latin = write("latin.json", b"[{\"description\": \"caf\xe9\"}]");
    let bom = write("bom.json", b"\xef\xbb\xbf[]");
    let refusals = [
        ("json", "JSON import"),
        ("btcpay_json", "BTCPay JSON export"),
        ("wasabi_bundle", "Wasabi bundle"),
    ];
    for file in [&csv, &latin, &bom] {
        for (format, label) in refusals {
            let error = match format {
                "json" => generic_json::load(file).err(),
                "btcpay_json" => btcpay::load(file, true).err(),
                _ => wasabi::load(file).err(),
            }
            .unwrap();
            assert_eq!(
                (
                    error.code.as_str(),
                    error.message.as_str(),
                    error.hint.as_deref()
                ),
                (
                    "validation",
                    format!("{label} is not valid UTF-8 JSON").as_str(),
                    Some("Choose the import format that matches the file.")
                ),
                "{file} as {format}"
            );
        }
    }
    // RecursionError is not a ValueError: it stays an internal error.
    let deep = write("deep.json", "[".repeat(1100).as_bytes());
    assert_eq!(generic_json::load(&deep).err().unwrap().code, "internal_error");
    // A missing file keeps the coordinator's own refusal.
    let missing = dir.join("absent.json").to_string_lossy().into_owned();
    assert_eq!(generic_json::load(&missing).err().unwrap().code, "not_found");
    assert_eq!(btcpay::load(&missing, true).err().unwrap().code, "not_found");
    std::fs::remove_dir_all(dir).unwrap();
}
