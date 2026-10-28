use super::*;

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
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
            phoenix,
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
            "kassiber-import-vector-{}-{}.csv",
            std::process::id(),
            name.bytes().map(u32::from).sum::<u32>()
        ));
        std::fs::write(
            &path,
            bytes
                .as_deref()
                .unwrap_or_else(|| case["text"].as_str().unwrap().as_bytes()),
        )
        .unwrap();
        let result = csv::load(path.to_str().unwrap()).and_then(|rows| {
            let rows = if case["format"] == "phoenix_csv" {
                phoenix::parse(rows)?
            } else {
                rows
            };
            rows.iter()
                .map(|row| {
                    let r = normalize(row)?;
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
            Ok(records) => assert_eq!(json!(records), case["records"], "{name}"),
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
