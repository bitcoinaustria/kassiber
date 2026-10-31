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
            "kassiber-import-vector-{}-{:?}-{}.csv",
            std::process::id(),
            std::thread::current().id(),
            name.bytes().map(u32::from).sum::<u32>()
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
            "phoenix_csv" => Format::Phoenix,
            "river_csv" => Format::River,
            "bullbitcoin_csv" => Format::Bull,
            "coinfinity_csv" => Format::Coinfinity,
            "21bitcoin_csv" => Format::TwentyOne,
            "strike_csv" => Format::Strike,
            "pocketbitcoin_csv" => Format::Pocket,
            _ => Format::Csv,
        };
        let result = format.load(path.to_str().unwrap()).and_then(|rows| {
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
        std::fs::remove_file(path).unwrap();
        match result {
            Ok(records) => assert_eq!(
                kassiber_pytext::loads(&json!(records).to_string()).unwrap(),
                kassiber_pytext::loads(&case["records"].to_string()).unwrap(),
                "{name}"
            ),
            Err(error) => assert_eq!(
                json!({"code":error.code,"message":error.message,"hint":error.hint}),
                case["error"],
                "{name}"
            ),
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
