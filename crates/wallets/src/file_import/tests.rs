use super::*;

pub(super) fn fixture() -> Connection {
    fixture_codec(false)
}
pub(super) fn fixture_codec(encrypted: bool) -> Connection {
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
pub(super) fn scalar(conn: &Connection, sql: &str) -> i64 {
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
    import_into(conn, content, format, "Source")
}
/// One file import of `content` into `wallet` (empty: the provider default).
pub(super) fn import_into(
    conn: &Connection,
    content: &str,
    format: Format,
    wallet: &str,
) -> Result<Py, AppError> {
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
            wallet: wallet.into(),
            file: file.to_str().unwrap().into(),
            format,
            mode: None,
            companion: None,
            timezone: None,
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
fn csv_import_records_each_created_rows_source_position() {
    // Python's record_batch numbers the rows a run created in file order;
    // rows a run only skips keep the position of the run that created them.
    let conn = fixture();
    import(
        &conn,
        "date,txid,direction,asset,amount,fee\n2025-01-02,b,in,BTC,1,0\n2025-01-02,a,in,BTC,1,0\n",
    )
    .unwrap();
    import(&conn, "date,txid,direction,asset,amount,fee\n2025-01-02,c,in,BTC,1,0\n2025-01-02,b,in,BTC,1,0\n2025-01-02,a,in,BTC,1,0\n2025-01-02,d,in,BTC,1,0\n").unwrap();
    let positions: Vec<(String, i64, i64)> = conn
        .prepare("SELECT t.external_id,b.import_sequence,l.source_position FROM import_batch_transactions l \
                  JOIN import_batches b ON b.id=l.batch_id JOIN transactions t ON t.id=l.transaction_id \
                  ORDER BY b.import_sequence,l.source_position")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        positions,
        [
            ("b".into(), 1, 1),
            ("a".into(), 1, 2),
            ("c".into(), 2, 1),
            ("d".into(), 2, 2)
        ]
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
        let outcome = import_vector_outcome(case);
        match outcome.get("records") {
            Some(records) => assert_eq!(
                kassiber_pytext::loads(&records.to_string()).unwrap(),
                kassiber_pytext::loads(&case["records"].to_string()).unwrap(),
                "{name}"
            ),
            None => {
                if let Some(expected) = case["error"].get("details_json") {
                    assert_eq!(outcome["error"]["details_json"], *expected, "{name}");
                }
                for key in ["code", "message", "hint"] {
                    assert_eq!(outcome["error"][key], case["error"][key], "{name}: {key}");
                }
            }
        }
    }
}

/// What the current code makes of one import vector: `{"records": [...]}`,
/// or `{"error": {...}}` with `details_json` when the vector records one.
fn import_vector_outcome(case: &Json) -> Json {
    {
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
            "bullbitcoin_wallet_csv" => Format::BullWallet,
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
                    if let Some(error) = &r.deferred { return Err(error.clone()); }
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
            Ok(records) => json!({ "records": records }),
            Err(error) => {
                let mut out = json!({"code":error.code,"message":error.message.replace(path.to_str().unwrap(),"{file}"),"hint":error.hint});
                if case["error"].get("details_json").is_some() {
                    let details = error
                        .details
                        .as_ref()
                        .map(|v| kassiber_pyjson::dumps(v, &Default::default()).unwrap())
                        .unwrap_or_else(|| "null".into());
                    out["details_json"] = json!(details);
                }
                json!({ "error": out })
            }
        }
    }
}

/// `KASSIBER_GOLDEN_WRITE=1` (refused under `CI`): rewrite a frozen import
/// vector file from the current code. Unchanged cases keep their bytes; a
/// changed record keeps the key order of the file's existing records.
fn write_import_vectors(
    path: &str,
    cases: &Json,
    networks: impl Fn(&Json) -> Option<Json>,
) -> bool {
    if std::env::var_os("KASSIBER_GOLDEN_WRITE").is_none() {
        return false;
    }
    assert!(
        std::env::var_os("CI").is_none(),
        "golden writes are refused under CI"
    );
    use kassiber_pyjson::{Key, Value as V};
    let text = std::fs::read_to_string(path).unwrap();
    let mut document = kassiber_pytext::loads(&text).unwrap();
    let py = |value: &Json| kassiber_pytext::loads(&value.to_string()).unwrap();
    let field_order: Vec<Key> = match &document {
        V::List(items) => items
            .iter()
            .find_map(|case| match kassiber_envelope::get(case, "records") {
                Some(V::List(records)) => {
                    records
                        .first()
                        .and_then(|r| match kassiber_envelope::get(r, "fields") {
                            Some(V::Dict(pairs)) => {
                                Some(pairs.iter().map(|(k, _)| k.clone()).collect())
                            }
                            _ => None,
                        })
                }
                _ => None,
            })
            .unwrap_or_default(),
        _ => unreachable!(),
    };
    let ordered = |value: V| -> V {
        let V::List(records) = value else {
            return value;
        };
        V::List(
            records
                .into_iter()
                .map(|record| {
                    let V::Dict(pairs) = record else {
                        return record;
                    };
                    V::Dict(
                        pairs
                            .into_iter()
                            .map(|(key, value)| match (key == Key::from("fields"), value) {
                                (true, V::Dict(mut fields)) => {
                                    fields.sort_by_key(|(k, _)| {
                                        field_order
                                            .iter()
                                            .position(|o| o == k)
                                            .unwrap_or(usize::MAX)
                                    });
                                    (key, V::Dict(fields))
                                }
                                (_, value) => (key, value),
                            })
                            .collect(),
                    )
                })
                .collect(),
        )
    };
    let V::List(items) = &mut document else {
        unreachable!()
    };
    for (item, case) in items.iter_mut().zip(cases.as_array().unwrap()) {
        let outcome = import_vector_outcome(case);
        let V::Dict(pairs) = item else { unreachable!() };
        let same = |key: &str| {
            outcome.get(key).is_some_and(|value| {
                pairs
                    .iter()
                    .any(|(k, old)| *k == Key::from(key) && *old == py(value))
            })
        };
        let networks = networks(case);
        let networks_same = networks.as_ref().is_none_or(|n| {
            pairs
                .iter()
                .any(|(k, old)| *k == Key::from("networks") && *old == py(n))
        });
        if (same("records") || same("error")) && networks_same {
            continue;
        }
        pairs.retain(|(k, _)| {
            ![
                Key::from("records"),
                Key::from("error"),
                Key::from("networks"),
            ]
            .contains(k)
        });
        match outcome.get("records") {
            Some(records) => {
                pairs.push((Key::from("records"), ordered(py(records))));
                if let Some(networks) = networks {
                    pairs.push((Key::from("networks"), py(&networks)));
                }
            }
            None => pairs.push((Key::from("error"), py(&outcome["error"]))),
        }
    }
    let options = kassiber_pyjson::Options {
        indent: Some(kassiber_pyjson::Indent::Spaces(2)),
        ..Default::default()
    };
    std::fs::write(
        path,
        kassiber_pyjson::dumps(&document, &options).unwrap() + "\n",
    )
    .unwrap();
    true
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
    // An exact price below 1e19 whose float mirror rounds up to it is out of
    // the engine's range too (d444f4db5).
    for (rate, message) in [
        (
            "9999999999999999999.5",
            "fiat_rate is outside the supported decimal range",
        ),
        ("1e19", "fiat_rate is outside the supported decimal range"),
    ] {
        let error = run(json!([{"date":"2025-01-03","txid":"mirror","amount":1,"fiat_rate":rate}]))
            .unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("validation", message),
            "{rate}"
        );
    }
    // The recalculated total obeys the shared price bounds (f823b4729).
    run(json!([{"date":"2025-01-03","txid":"bounded","amount":1,"kind":"routing_income","fiat_rate":"1e17"}])).unwrap();
    let error =
        run(json!([{"date":"2025-01-03","txid":"bounded","amount":1000,"kind":"routing_income"}]))
            .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (
            "validation",
            "fiat_value is outside the supported decimal range"
        )
    );
    run(json!([{"date":"2025-01-03","txid":"unpriced","amount":1,"kind":"routing_income","fiat_value":10}])).unwrap();
    // A value-only price stays a total assertion: no unit rate is stored,
    // so the quantity correction below cannot rescale it.
    assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE external_id='unpriced' AND fiat_rate IS NULL AND fiat_rate_exact IS NULL AND fiat_value_exact='10'"),1);
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
fn python_bull_wallet_import_vectors() {
    let cases: Json = serde_json::from_str(include_str!(
        "../../tests/vectors/python_bull_wallet_imports.json"
    ))
    .unwrap();
    let networks = |case: &Json| {
        let text = case["text"].as_str()?;
        let rows = bull_wallet::parse(csv::rows(text).ok()?).ok()?;
        Some(json!(rows
            .iter()
            .map(|row| row.data["_bullbitcoin_wallet_network"].clone())
            .collect::<Vec<_>>()))
    };
    if write_import_vectors(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/vectors/python_bull_wallet_imports.json"
        ),
        &cases,
        networks,
    ) {
        return;
    }
    assert_import_vectors(&cases);
    // The wallet's network filter reads each row's normalized network, and
    // enrichment matches descriptor rows whatever fee Bull reports.
    for case in cases.as_array().unwrap() {
        let Some(networks) = case.get("networks") else {
            continue;
        };
        let parsed =
            || bull_wallet::parse(csv::rows(case["text"].as_str().unwrap()).unwrap()).unwrap();
        let rows = parsed();
        let seen: Vec<&str> = rows
            .iter()
            .map(|row| row.data["_bullbitcoin_wallet_network"].as_str().unwrap())
            .collect();
        assert_eq!(json!(seen), *networks, "{}", case["name"]);
        for row in &rows {
            assert!(normalize(row).unwrap().ignore_fee, "{}", case["name"]);
        }
        for network in ["bitcoin", "liquid", "lightning"] {
            let kept = bull_wallet::filter(parsed(), Some(network)).len();
            assert_eq!(kept, seen.iter().filter(|n| **n == network).count());
        }
        assert_eq!(bull_wallet::filter(parsed(), None).len(), rows.len());
    }
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
        companion: None,
        timezone: None,
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
                companion: None,
                timezone: None,
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
        companion: None,
        timezone: None,
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
        companion: None,
        timezone: None,
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
            companion: None,
            timezone: None,
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
    let rows = coinfinity::parse(csv::rows("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,0,match\n").unwrap(), None, &mut Default::default()).unwrap();
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
        companion: None,
        timezone: None,
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
                companion: None,
                timezone: None,
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
    let rows = coinfinity::parse(csv::rows("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,1,\n").unwrap(), None, &mut Default::default()).unwrap();
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
        companion: None,
        timezone: None,
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
        companion: None,
        timezone: None,
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
    let rows = coinfinity::parse(csv::rows("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,1,\n").unwrap(), None, &mut Default::default()).unwrap();
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
        let rows =
            coinfinity::parse(csv::rows(&input).unwrap(), None, &mut Default::default()).unwrap();
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
    let rows=coinfinity::parse(csv::rows("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,1,\n").unwrap(), None, &mut Default::default()).unwrap();
    let request = Request {
        profile_id: "p".into(),
        wallet: String::new(),
        file: "coinfinity.csv".into(),
        format: Format::Coinfinity,
        mode: None,
        companion: None,
        timezone: None,
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
        companion: None,
        timezone: None,
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
        companion: None,
        timezone: None,
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
        companion: None,
        timezone: None,
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
        companion: None,
        timezone: None,
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
            companion: None,
            timezone: None,
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
        companion: None,
        timezone: None,
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
            companion: None,
            timezone: None,
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
        companion: None,
        timezone: None,
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
            companion: None,
            timezone: None,
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
            companion: None,
            timezone: None,
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
const BINANCE_REF: &str = "binance-supplemental:autoinvest:economic:21289c95049c68b07fe8e59a91647df8732abcd79ff602f39291ebe6b0f23f41:1";
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
            companion: None,
            timezone: None,
        },
        None,
    );
    std::fs::remove_file(file).unwrap();
    result
}
/// A BTC autoinvest row keeps the identity the Python reference gave it now
/// that the identity reads every row in the `crypto` scope: the current id
/// and a legacy file-position id resolve to the frozen economic key.
#[test]
fn binance_autoinvest_identity_stays_byte_identical_for_btc_rows() {
    let conn = fixture();
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    binance_import(&conn, BINANCE_AUTO, "full", "Source").unwrap();
    let (external, raw): (String, String) = conn
        .query_row(
            "SELECT external_id,raw_json FROM transactions WHERE wallet_id='wallet'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(external, BINANCE_REF);
    let economic = BINANCE_REF.strip_suffix(":1").unwrap().to_owned();
    assert_eq!(
        synthetic_csv_identity(&external, &raw),
        Some((economic.clone(), 1, false))
    );
    assert_eq!(
        synthetic_csv_identity(
            "binance-supplemental:2025-01-02 03:04:05:autoinvest:BTC:0.002:3",
            &raw
        ),
        Some((economic, 3, true))
    );
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
        // The quote amount is part of an ID-less row's economic identity, so
        // a changed price is a different trade, not an enrichment.
        assert_eq!(
            kassiber_envelope::get(&enriched, "imported"),
            Some(&Py::from(1))
        );
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE wallet_id='wallet' AND fiat_value_exact='201.00' AND fiat_rate_exact='100500'"),1);
        conn.execute(
            "UPDATE transactions SET external_id_kind='txid' WHERE wallet_id='chain'",
            [],
        )
        .unwrap();
        binance_import(&conn, BINANCE_AUTO, "full", "Source").unwrap();
        // The original row and the differently priced trade.
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transactions WHERE wallet_id='wallet' AND external_id_kind IS NULL"),2);
        conn.execute("DELETE FROM transactions WHERE wallet_id='chain'", [])
            .unwrap();
        let unmatched = binance_import(&conn, BINANCE_AUTO, "full", "Source").unwrap();
        assert_eq!(
            kassiber_envelope::get(&unmatched, "unmatched"),
            Some(&Py::from(1))
        );
        // Both trades, the second imported earlier without a chain match.
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='binance-wallet-gap'"),2);
        assert_eq!(scalar(&conn,"SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE code='binance-matched'"),0);
        // Trailing zeros do not change an economic identity.
        let scaled = BINANCE_AUTO.replace("0.002 BTC", "0.0020 BTC");
        assert_eq!(
            kassiber_envelope::get(
                &binance_import(&conn, &scaled, "full", "Source").unwrap(),
                "imported"
            ),
            Some(&Py::from(0))
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
                    companion: None,
                    timezone: None,
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
fn btcpay_metadata_binds_to_the_stored_text_identity() {
    // 464f82094: `str(txid)`, so a boolean identity annotates the row stored
    // as 'True' and never the numeric '1' SQLite would bind True to, and a
    // list identity binds its repr instead of failing the import.
    let conn = fixture_codec(false);
    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,created_at) SELECT 'numeric',workspace_id,profile_id,id,'numeric-fp','1','BTC','inbound',1,0,'2025-01-01T00:00:00Z','2025-01-01' FROM wallets LIMIT 1", []).unwrap();
    for (identity, stored) in [
        (json!(true), "True"),
        (
            json!(["one", {"text": "two"}, null, false]),
            "['one', {'text': 'two'}, None, False]",
        ),
    ] {
        let body = json!([{"TransactionId":identity,"Timestamp":"2025-01-02","Amount":"0.001 BTC","Comment":"receipt"}]).to_string();
        let result = import_kind(&conn, &body, Format::BtcpayJson).unwrap();
        assert_eq!(
            kassiber_envelope::get(&result, "btcpay_notes_set"),
            Some(&Py::from(1))
        );
        let note: Option<String> = conn
            .query_row(
                "SELECT note FROM transactions WHERE external_id=?",
                [stored],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(note.as_deref(), Some("receipt"), "{stored}");
    }
    let numeric: Option<String> = conn
        .query_row(
            "SELECT note FROM transactions WHERE id='numeric'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(numeric, None);
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
fn btcpay_metadata_lookup_is_the_untrimmed_string_identity_and_scoped() {
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
        // 464f82094: each identity binds as the text it is stored under, so
        // the boolean annotates 'True' and the integer its own '1'.
        assert_eq!(
            kassiber_envelope::get(&result, "btcpay_notes_set"),
            Some(&Py::from(4))
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
                "SELECT count(*) FROM transactions WHERE external_id='True' AND note='boolean'"
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
                "SELECT count(*) FROM transactions WHERE external_id='1' AND note='integer'"
            ),
            1
        );
        // A list id is no binding error: the row is stored and its metadata
        // found under the same str() identity.
        let body = json!([{"TransactionId":["invalid"],"Timestamp":"2025-01-02","Amount":"1 BTC","Comment":"note"}]).to_string();
        let result = import_kind(&conn, &body, Format::BtcpayJson).unwrap();
        assert_eq!(
            kassiber_envelope::get(&result, "btcpay_notes_set"),
            Some(&Py::from(1))
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM transactions WHERE external_id='[''invalid'']' AND note='note'"
            ),
            1
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 7);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 2);
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
        // A bundle file is a rollback batch like any file import (06ddfc545).
        let batch = kassiber_envelope::get(&result, "import_batch_id").unwrap();
        let batch = kassiber_envelope::py_str(batch);
        assert_eq!(
            scalar(
                &conn,
                &format!("SELECT count(*) FROM import_batches WHERE id='{batch}' AND source_format='wasabi_bundle' AND rows_inserted=1 AND rows_updated=0 AND rows_skipped=0")
            ),
            1
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM import_batch_transactions JOIN transactions t ON t.id=transaction_id"
            ),
            1
        );
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
        // A run that created nothing leaves no batch to roll back.
        assert!(kassiber_envelope::get(&repeated, "import_batch_id").is_none());
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
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
                companion: None,
                timezone: None,
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
    assert_eq!(
        generic_json::load(&deep).err().unwrap().code,
        "internal_error"
    );
    // A missing file keeps the coordinator's own refusal.
    let missing = dir.join("absent.json").to_string_lossy().into_owned();
    assert_eq!(
        generic_json::load(&missing).err().unwrap().code,
        "not_found"
    );
    assert_eq!(
        btcpay::load(&missing, true).err().unwrap().code,
        "not_found"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn record_imports_match_existing_only_and_label_their_source() {
    let conn = fixture();
    let rows = |amount: &str| {
        csv::rows(&format!(
            "txid,occurred_at,direction,asset,amount,fee,description\nshared,2025-01-02T00:00:00Z,inbound,BTC,{amount},0,first\nfresh,2025-01-03T00:00:00Z,inbound,BTC,0.2,0,second\n"
        ))
        .unwrap()
    };
    let spec = |match_existing_only| RecordImport {
        profile_id: "p",
        wallet_id: "wallet",
        label: "bullbitcoin-wallet:bitcoin",
        match_existing_only,
        report_updates: true,
        btcpay: false,
        settlement_ids: &[],
    };
    let only = import_records(&conn, &spec(true), &rows("0.1")).unwrap();
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transactions"),
        0,
        "never inserts"
    );
    assert_eq!(kassiber_envelope::get(&only, "skipped"), Some(&Py::from(2)));
    assert_eq!(
        kassiber_envelope::get(&only, "source"),
        Some(&s("bullbitcoin-wallet:bitcoin"))
    );
    let inserted = import_records(&conn, &spec(false), &csv::rows("txid,occurred_at,direction,asset,amount,fee\nshared,2025-01-02T00:00:00Z,inbound,BTC,0.1,0\n").unwrap()).unwrap();
    assert_eq!(
        kassiber_envelope::get(&inserted, "imported"),
        Some(&Py::from(1))
    );
    let enriched = import_records(&conn, &spec(true), &rows("0.1")).unwrap();
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    assert_eq!(
        kassiber_envelope::get(&enriched, "updated"),
        Some(&Py::from(1))
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE description='first'"
        ),
        1
    );
    let error = import_records(
        &conn,
        &RecordImport {
            wallet_id: "missing",
            ..spec(true)
        },
        &rows("0.1"),
    )
    .unwrap_err();
    assert_eq!(error.code, "not_found");
}

// Bull Bitcoin wallet-history export (bullbitcoin_wallet_csv): formatter
// generations A (v6.11), B (v6.12) and C (v6.13+) share one header.
const BULL_HEADER: &str = "date,type,direction,amount_sats,amount_btc,fee_sats,status,txid,network,address,swap_id,preimage,total_swap_fees_sats,send_network,receive_network,send_txid,receive_txid\n";

fn bull_txid(digit: char) -> String {
    std::iter::repeat_n(digit, 64).collect()
}

fn bull_csv(rows: &[String]) -> String {
    let mut text = BULL_HEADER.to_owned();
    for row in rows {
        text.push_str(row);
        text.push('\n');
    }
    text
}

fn bull_received(digit: char) -> String {
    format!(
        "2026-06-01T09:15:42Z,onchain,incoming,250000,0.00250000,0,confirmed,{},bitcoin,bc1qexample,,,,,,,",
        bull_txid(digit)
    )
}

fn bull_chain_swap_out(fee: u32) -> String {
    let send = bull_txid('4');
    format!("2026-06-03T10:00:00Z,chain_swap,outgoing,500000,0.00500000,{fee},completed,{send},bitcoin,lq1qexample,swap4,,3500,bitcoin,liquid,{send},{}", bull_txid('5'))
}

fn bull_payjoin_send(amount: u32) -> String {
    format!(
        "2026-06-05T12:00:00Z,payjoin_send,outgoing,{amount},0.00{amount},2381,completed,{},bitcoin,bc1qmerchant,,,,,,,",
        bull_txid('6')
    )
}

fn bull_import(conn: &Connection, rows: &[String]) -> Result<Py, AppError> {
    import_kind(conn, &bull_csv(rows), Format::BullWallet)
}

fn bull_network(conn: &Connection, network: &str) {
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id='wallet'",
        [format!(r#"{{"bullbitcoin_wallet_network":"{network}"}}"#)],
    )
    .unwrap();
}

fn bull_parsed(rows: &[String]) -> Result<Vec<Record>, AppError> {
    bull_wallet::parse(csv::rows(&bull_csv(rows))?)?
        .iter()
        .map(normalize)
        .collect()
}

#[test]
fn bull_wallet_generations_parse_and_formula_guard_is_stripped_exactly() {
    // Generation A and B rows parse alike; B's `refunded` rows are skipped.
    let a = bull_parsed(&[bull_received('1'), bull_chain_swap_out(2100)]).unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(string(&a[0].fields, "external_id"), bull_txid('1'));
    assert_eq!(a[0].fields["amount"], Sql::Integer(250_000_000));
    assert_eq!(a[1].fields["fee"], Sql::Integer(2_100_000));
    assert!(!a[1].bull.as_ref().unwrap().generation_c);
    assert!(!string(&a[0].fields, "raw_json").contains("export_generation"));
    let refunded = bull_chain_swap_out(2100).replace("completed", "refunded");
    assert_eq!(bull_parsed(&[refunded]).unwrap().len(), 0);
    // Generation C: the `'` formula guard goes, nothing else does.
    let guarded = bull_received('1').replace("bc1qexample", "'=HYPERLINK(1)");
    let c = bull_parsed(&[guarded, bull_chain_swap_out(1410)]).unwrap();
    assert!(string(&c[0].fields, "raw_json").contains(r#""address": "=HYPERLINK(1)""#));
    assert!(string(&c[0].fields, "raw_json").contains(r#""export_generation": "v6.13+""#));
    assert!(c[1].bull.as_ref().unwrap().generation_c);
    let quoted = bull_received('1').replace("bc1qexample", "'bc1q");
    let kept = bull_parsed(&[quoted]).unwrap();
    assert!(string(&kept[0].fields, "raw_json").contains(r#""address": "'bc1q""#));
    assert!(!kept[0].bull.as_ref().unwrap().generation_c);
    // `aborted` (v6.13+ only) also proves the generation.
    let aborted = bull_payjoin_send(100001).replace("completed", "aborted");
    let rows = bull_parsed(&[aborted, bull_chain_swap_out(1410)]).unwrap();
    assert!(rows.iter().all(|r| r.bull.as_ref().unwrap().generation_c));
    // A guarded or accounting-style negative count is held for review with
    // its magnitude and stated direction; the raw row keeps the sign.
    for amount in ["'-150", "-150", "(150)"] {
        let row = bull_received('1').replacen("250000", amount, 1);
        let held = bull_parsed(&[row]).unwrap();
        assert_eq!(held[0].fields["amount"], Sql::Integer(150_000), "{amount}");
        assert_eq!(string(&held[0].fields, "direction"), "inbound", "{amount}");
        assert!(held[0].bull.as_ref().unwrap().negative(), "{amount}");
        let raw = string(&held[0].fields, "raw_json");
        assert!(
            raw.contains(r#""negative_columns": ["amount_sats"]"#),
            "{amount}"
        );
        assert!(
            raw.contains(&format!(
                r#""amount_sats": "{}""#,
                amount.trim_start_matches('\'')
            )),
            "{amount}"
        );
    }
    let row = bull_received('1').replacen(",0,confirmed", ",'-2,confirmed", 1);
    let held = bull_parsed(&[row]).unwrap();
    assert_eq!(held[0].fields["fee"], Sql::Integer(2_000));
    assert!(string(&held[0].fields, "raw_json").contains(r#""negative_columns": ["fee_sats"]"#));
    assert!(held[0].bull.as_ref().unwrap().negative());
    // `-0` is zero, not negative.
    let zero = bull_received('1').replacen("250000", "-0", 1);
    assert_eq!(
        bull_parsed(&[zero]).unwrap()[0]
            .bull
            .as_ref()
            .unwrap()
            .review,
        None
    );
}

#[test]
fn bull_wallet_negative_rows_are_held_for_review_and_never_change_a_holding() {
    let conn = fixture();
    bull_network(&conn, "bitcoin");
    let negative = bull_received('2').replacen("250000", "-5000", 1);
    // A self-transfer stating a negative count is held too, not skipped.
    let own = format!(
        "2026-06-04T08:00:00Z,onchain,self,-40000,0.00040000,0,confirmed,{},bitcoin,bc1qown,,,,,,,",
        bull_txid('3')
    );
    let result = bull_import(&conn, &[bull_received('1'), negative.clone(), own]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(3))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "bullbitcoin_wallet_review"),
        Some(&Py::from(2))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "bullbitcoin_wallet_skipped"),
        None
    );
    let Some(Py::List(warnings)) = kassiber_envelope::get(&result, "warnings") else {
        panic!("warnings: {result:?}")
    };
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(kassiber_envelope::py_str(&warnings[0])
        .contains("Held 2 Bull Bitcoin wallet row(s) stating a negative amount or fee"));
    let (direction, amount, excluded, raw): (String, i64, i64, String) = conn
        .query_row(
            "SELECT direction,amount,excluded,raw_json FROM transactions WHERE external_id=?",
            [bull_txid('2')],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (direction.as_str(), amount, excluded),
        ("inbound", 5_000_000, 1)
    );
    assert!(raw.contains(r#""amount_sats": "-5000""#), "{raw}");
    assert!(
        raw.contains(r#""negative_columns": ["amount_sats"]"#),
        "{raw}"
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transactions WHERE excluded=0"),
        1,
        "only the plain receive books"
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE t.code IN ('bullbitcoin-wallet-review','bullbitcoin-wallet-negative-amount')"),
        4
    );
    // Re-importing leaves the held row, and the user's inclusion, alone.
    conn.execute_batch("UPDATE transactions SET excluded=0,description='checked'")
        .unwrap();
    let again = bull_import(&conn, &[negative]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&again, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(
        kassiber_envelope::get(&again, "bullbitcoin_wallet_skipped"),
        Some(&kassiber_envelope::dict([("negative_amount", Py::from(1))]))
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transactions WHERE excluded=1"),
        0
    );
    // A negative row never changes a transaction the wallet observed, not
    // even metadata a plain Bull row would fill.
    conn.execute_batch(&format!("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,external_id_kind,fingerprint,occurred_at,direction,asset,amount,fee,raw_json,created_at) VALUES('obs','ws','p','wallet','{}','txid','observed','2026-06-05T12:00:00Z','outbound','BTC',99705000,2381000,'{{\"txid\":\"x\"}}','2026-06-05')", bull_txid('6'))).unwrap();
    let observed = bull_payjoin_send(100001).replacen(",100001,", ",-100001,", 1);
    let result = bull_import(&conn, &[observed]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "bullbitcoin_wallet_review"),
        None
    );
    let (amount, kind, boundary, raw, fingerprint): (i64, Option<String>, Option<String>, String, String) = conn
        .query_row(
            "SELECT amount,kind,privacy_boundary,raw_json,fingerprint FROM transactions WHERE id='obs'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        (amount, kind, boundary, raw.as_str(), fingerprint.as_str()),
        (99_705_000, None, None, r#"{"txid":"x"}"#, "observed")
    );
    let Some(Py::List(warnings)) = kassiber_envelope::get(&result, "warnings") else {
        panic!("warnings: {result:?}")
    };
    assert!(kassiber_envelope::py_str(&warnings[0]).contains("which they never change"));
    // Other refusals still surface past a negative row.
    let error = bull_import(
        &conn,
        &[
            bull_received('2').replacen("250000", "-5000", 1),
            bull_received('9').replace(",bitcoin,", ",signet,"),
        ],
    )
    .unwrap_err();
    assert_eq!(
        error.message,
        "Unsupported Bull Bitcoin wallet network 'signet'"
    );
}

#[test]
fn bull_wallet_negative_rows_never_enrich_or_prove_lightning() {
    let conn = fixture();
    conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('liquid','ws','p','Liquid','custom','{}','2025-01-01')").unwrap();
    let claim = bull_txid('7');
    conn.execute_batch(&format!("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,raw_json,created_at) VALUES('claim','ws','p','liquid','{claim}','f','2026-06-02T08:00:00Z','inbound','LBTC',79850000,0,'{{}}','2026-06-02')")).unwrap();
    let receive = format!("2026-06-02T08:00:00Z,lightning_receive,incoming,-80000,0.00080000,150,completed,{claim},lightning,,ln7,,420,,,,{claim}");
    let liquid = format!(
        "2026-06-02T08:00:00Z,liquid,incoming,79850,0.00079850,(5),completed,{claim},liquid,lq1q,,,,,,,"
    );
    // Enrichment only ever matches, so it skips and counts negative rows,
    // whether they would match directly or through a Lightning claim.
    let file =
        std::env::temp_dir().join(format!("kassiber-bull-negative-{}.csv", std::process::id()));
    std::fs::write(&file, bull_csv(&[receive.clone(), liquid])).unwrap();
    let rows = bull_wallet_rows(file.to_str().unwrap());
    std::fs::remove_file(&file).unwrap();
    let routed = bull_wallet_enrichment(rows.unwrap(), "liquid").unwrap();
    assert!(routed.rows.is_empty());
    assert_eq!(
        routed.skipped_py(),
        Some(kassiber_envelope::dict([("negative_amount", Py::from(2))]))
    );
    // A Liquid CSV wallet would only match the claim: skipped, not booked.
    bull_network(&conn, "liquid");
    let result = bull_import(&conn, std::slice::from_ref(&receive)).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "bullbitcoin_wallet_skipped"),
        Some(&kassiber_envelope::dict([("negative_amount", Py::from(1))]))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    // A Lightning wallet holds it, for the negative count first.
    bull_network(&conn, "lightning");
    let unseen = receive.replace(&claim, &bull_txid('d'));
    let held = bull_import(&conn, &[unseen]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&held, "bullbitcoin_wallet_review"),
        Some(&Py::from(1))
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE t.code='bullbitcoin-wallet-negative-amount'"),
        1
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM tags WHERE code='bullbitcoin-wallet-lightning-network-unproven'"
        ),
        0
    );
}

#[test]
fn bull_wallet_lightning_hash_comes_only_from_a_preimage() {
    let claim = bull_txid('7');
    let receive = format!("2026-06-02T08:00:00Z,lightning_receive,incoming,80000,0.00080000,150,completed,{claim},lightning,,ln7,,420,,,,{claim}");
    let preimage = bull_txid('8');
    let send = format!("2026-06-02T09:00:00Z,lightning_send,outgoing,60000,0.00060000,40,completed,{},lightning,,ln9,{preimage},300,,,{},", bull_txid('9'), bull_txid('9'));
    let rows = bull_parsed(&[receive, send.clone(), send.replace(&preimage, "zz")]).unwrap();
    assert_eq!(rows[0].fields["payment_hash"], Sql::Null);
    assert_eq!(rows[0].fields["payment_hash_source"], Sql::Null);
    use sha2::Digest;
    let hash: String = sha2::Sha256::digest([0x88u8; 32])
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(rows[1].fields["payment_hash"], Sql::Text(hash));
    assert!(!string(&rows[1].fields, "raw_json").contains(&preimage));
    assert_eq!(
        rows[2].fields["payment_hash"],
        Sql::Null,
        "the txid is no hash"
    );
}

#[test]
fn bull_wallet_undated_pending_rows_are_skipped_and_reported() {
    let conn = fixture();
    bull_network(&conn, "bitcoin");
    let undated = format!(
        ",onchain,incoming,30000,0.00030000,0,pending,{},bitcoin,bc1qexample,,,,,,,",
        bull_txid('2')
    );
    let undated_liquid = format!(
        ",liquid,incoming,30000,0.00030000,0,pending,{},liquid,lq1qexample,,,,,,,",
        bull_txid('a')
    );
    let result = bull_import(
        &conn,
        &[bull_received('1'), undated.clone(), undated_liquid],
    )
    .unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "bullbitcoin_wallet_skipped"),
        Some(&kassiber_envelope::dict([("undated_pending", Py::from(1))])),
        "only this wallet's network counts"
    );
    let Some(Py::List(warnings)) = kassiber_envelope::get(&result, "warnings") else {
        panic!("warnings: {result:?}")
    };
    assert!(kassiber_envelope::py_str(&warnings[0]).contains("without a date"));
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    // An undated row that claims to be confirmed is malformed: it still fails.
    let error = bull_import(&conn, &[undated.replace("pending", "confirmed")]).unwrap_err();
    assert_eq!(error.message, "Missing occurred_at/date value");
}

#[test]
fn bull_wallet_self_transfer_books_its_miner_fee_once() {
    let conn = fixture();
    bull_network(&conn, "bitcoin");
    let own = format!(
        "2026-06-04T08:00:00Z,onchain,self,40000,0.00040000,312,confirmed,{},bitcoin,bc1qown,,,,,,,",
        bull_txid('3')
    );
    let free = own
        .replace(",312,", ",0,")
        .replace(&bull_txid('3'), &bull_txid('b'));
    let result = bull_import(&conn, &[own.clone(), free]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "bullbitcoin_wallet_skipped"),
        Some(&kassiber_envelope::dict([(
            "self_without_fee",
            Py::from(1)
        )]))
    );
    let (direction, amount, fee, kind): (String, i64, i64, String) = conn
        .query_row(
            "SELECT direction,amount,fee,kind FROM transactions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (direction.as_str(), amount, fee, kind.as_str()),
        ("outbound", 0, 312_000, "fee"),
        "a fee-only outflow, as an observer books a send-to-self"
    );
    let again = bull_import(&conn, &[own]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&again, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
}

#[test]
fn bull_wallet_payjoin_without_observation_is_held_for_review_and_keeps_the_net() {
    let conn = fixture();
    bull_network(&conn, "bitcoin");
    // v6.13+ reports the negotiated 100001 sats; the wallet's debit was
    // 99705 + 2381 = 102086 sats.
    let first = bull_import(&conn, &[bull_payjoin_send(100001)]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&first, "bullbitcoin_wallet_review"),
        Some(&Py::from(1))
    );
    assert_eq!(scalar(&conn, "SELECT excluded FROM transactions"), 1);
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE t.code IN ('bullbitcoin-wallet-review','bullbitcoin-wallet-payjoin-amount-unverified')"),
        2
    );
    // A v6.11/v6.12 export of the same txid names the smaller wallet net.
    let older = bull_import(&conn, &[bull_payjoin_send(99705)]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&older, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
    assert_eq!(scalar(&conn, "SELECT amount FROM transactions"), 99_705_000);
    let fingerprint: String = conn
        .query_row("SELECT fingerprint FROM transactions", [], |r| r.get(0))
        .unwrap();
    // The nominal amount never replaces the net again, and a user's
    // inclusion survives a re-import.
    conn.execute_batch("UPDATE transactions SET excluded=0")
        .unwrap();
    bull_import(&conn, &[bull_payjoin_send(100001)]).unwrap();
    assert_eq!(scalar(&conn, "SELECT amount FROM transactions"), 99_705_000);
    assert_eq!(scalar(&conn, "SELECT excluded FROM transactions"), 0);
    let kept: String = conn
        .query_row("SELECT fingerprint FROM transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        kept, fingerprint,
        "the fingerprint keeps hashing the stored amount"
    );
}

#[test]
fn bull_wallet_observed_transaction_keeps_its_amounts() {
    let conn = fixture();
    bull_network(&conn, "bitcoin");
    // A row the wallet observed itself (not a Bull row): debit 102086 sats.
    conn.execute_batch(&format!("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,external_id_kind,fingerprint,occurred_at,direction,asset,amount,fee,kind,raw_json,created_at) VALUES('obs','ws','p','wallet','{}','txid','observed','2026-06-05T12:00:00Z','outbound','BTC',99705000,2381000,'withdrawal','{{\"txid\":\"x\"}}','2026-06-05')", bull_txid('6'))).unwrap();
    let result = bull_import(&conn, &[bull_payjoin_send(100001)]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(0))
    );
    let (amount, fee, excluded, boundary, fingerprint): (i64, i64, i64, String, String) = conn
        .query_row(
            "SELECT amount,fee,excluded,privacy_boundary,fingerprint FROM transactions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!((amount, fee, excluded), (99_705_000, 2_381_000, 0));
    assert_eq!(
        (boundary.as_str(), fingerprint.as_str()),
        ("payjoin", "observed")
    );
}

#[test]
fn bull_wallet_chain_swap_fee_upgrades_only_from_a_provable_v6_13_export() {
    let conn = fixture();
    bull_network(&conn, "bitcoin");
    bull_import(&conn, &[bull_chain_swap_out(2100)]).unwrap();
    // Without a generation marker 1410 might be an older provider figure.
    bull_import(&conn, &[bull_chain_swap_out(1410)]).unwrap();
    assert_eq!(scalar(&conn, "SELECT fee FROM transactions"), 2_100_000);
    let marker = format!(
        "2026-06-06T12:00:00Z,payjoin_receive,incoming,50000,0.00050000,0,aborted,{},bitcoin,bc1qown,,,,,,,",
        bull_txid('c')
    );
    bull_import(&conn, &[bull_chain_swap_out(1410), marker]).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT fee FROM transactions WHERE kind='chain_swap'"
        ),
        1_410_000
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE kind='chain_swap'"
        ),
        1
    );
}

#[test]
fn bull_wallet_lightning_rows_book_only_where_their_network_is_proven() {
    let conn = fixture();
    conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('liquid','ws','p','Liquid','custom','{}','2025-01-01')").unwrap();
    let claim = bull_txid('7');
    let receive = format!("2026-06-02T08:00:00Z,lightning_receive,incoming,80000,0.00080000,150,completed,{claim},lightning,,ln7,,420,,,,{claim}");
    let unseen = receive
        .replace(&claim, &bull_txid('d'))
        .replace("ln7", "ln8");
    // A Liquid CSV wallet cannot prove which wallet paid a Lightning swap.
    bull_network(&conn, "liquid");
    let result = bull_import(&conn, &[receive.clone(), unseen.clone()]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(
        kassiber_envelope::get(&result, "bullbitcoin_wallet_skipped"),
        Some(&kassiber_envelope::dict([(
            "lightning_not_observed_in_wallet",
            Py::from(2)
        )]))
    );
    // Another wallet observed the claim: that wallet holds it.
    conn.execute_batch(&format!("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,raw_json,created_at) VALUES('claim','ws','p','liquid','{claim}','f','2026-06-02T08:00:00Z','inbound','LBTC',79850000,0,'{{}}','2026-06-02')")).unwrap();
    bull_network(&conn, "lightning");
    let held = bull_import(&conn, &[receive.clone(), unseen]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&held, "imported"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&held, "bullbitcoin_wallet_review"),
        Some(&Py::from(1))
    );
    assert_eq!(
        kassiber_envelope::get(&held, "bullbitcoin_wallet_skipped"),
        Some(&kassiber_envelope::dict([(
            "lightning_booked_in_other_wallet",
            Py::from(1)
        )]))
    );
    assert_eq!(
        scalar(
            &conn,
            &format!(
                "SELECT excluded FROM transactions WHERE wallet_id='wallet' AND external_id='{}'",
                bull_txid('d')
            )
        ),
        1
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE t.code='bullbitcoin-wallet-lightning-network-unproven'"),
        1
    );
    // Enrichment of the observing Liquid wallet proves the network by txid:
    // the observed L-BTC claim keeps its amount; the row fills its empty kind.
    let routed = bull_wallet_enrichment(
        bull_wallet_rows(&{
            let file = std::env::temp_dir()
                .join(format!("kassiber-bull-enrich-{}.csv", std::process::id()));
            std::fs::write(&file, bull_csv(&[receive])).unwrap();
            file.to_string_lossy().into_owned()
        })
        .unwrap(),
        "liquid",
    )
    .unwrap();
    std::fs::remove_file(
        std::env::temp_dir().join(format!("kassiber-bull-enrich-{}.csv", std::process::id())),
    )
    .unwrap();
    let enriched = import_records(
        &conn,
        &RecordImport {
            profile_id: "p",
            wallet_id: "liquid",
            label: "bullbitcoin-wallet:liquid",
            match_existing_only: true,
            report_updates: true,
            btcpay: false,
            settlement_ids: &[],
        },
        &routed.rows,
    )
    .unwrap();
    assert_eq!(
        kassiber_envelope::get(&enriched, "updated"),
        Some(&Py::from(1))
    );
    let (amount, kind): (i64, String) = conn
        .query_row(
            "SELECT amount,kind FROM transactions WHERE id='claim'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((amount, kind.as_str()), (79_850_000, "lightning_receive"));
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE wallet_id='liquid'"
        ),
        1
    );
}

#[test]
fn bull_wallet_lightning_row_never_inserts_beside_a_differently_shaped_holding() {
    let conn = fixture();
    bull_network(&conn, "liquid");
    let claim = bull_txid('7');
    // The wallet holds the txid, but as an outflow: the inbound Lightning
    // row proves nothing about it and must not become a second row.
    conn.execute_batch(&format!("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,raw_json,created_at) VALUES('held','ws','p','wallet','{claim}','f','2026-06-02T08:00:00Z','outbound','LBTC',1000,0,'{{}}','2026-06-02')")).unwrap();
    let receive = format!("2026-06-02T08:00:00Z,lightning_receive,incoming,80000,0.00080000,150,completed,{claim},lightning,,ln7,,420,,,,{claim}");
    let result = bull_import(&conn, &[receive]).unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "imported"),
        Some(&Py::from(0))
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 1);
}

const BISQ_TRADES: &str = include_str!("../../../../docs/exchanges/samples/bisq/tradeHistory.csv");
const BISQ_TRANSACTIONS: &str =
    include_str!("../../../../docs/exchanges/samples/bisq/transactions.csv");
const BISQ_HEADER: &str = "Trade ID,Date/Time,Market,Price,Deviation,Amount in BTC,Amount,Currency,Transaction Fee,Trade Fee BTC,Trade Fee BSQ,Buyer Deposit,Seller Deposit,Offer type,Status\n";
const BISQ_TX_HEADER: &str =
    "Date/Time,Trade ID,Details,Address,Transaction ID,Amount in BTC,Memo,Confirmations\n";
/// The sample deposits as Bisq dates them, with a separate fee.
const BISQ_DEPOSITS: &str = "a1 2024-03-15T14:41:10Z outbound 0.006 0.00005
c2 2024-04-02T09:15:00Z outbound 0.0575 0.00004";

fn bisq_txid(byte: &str) -> String {
    byte.repeat(32)
}
fn bisq_book() -> Connection {
    let conn = fixture();
    conn.execute_batch("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'; INSERT OR IGNORE INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','2025-01-01')").unwrap();
    conn
}
/// Rows the watch-only wallet observed, one per line: txid byte, time,
/// direction, BTC, fee.
fn bisq_onchain(conn: &Connection, rows: &str) {
    let mut content = "txid,occurred_at,direction,asset,amount,fee\n".to_owned();
    for line in rows.lines() {
        let f = line.split_whitespace().collect::<Vec<_>>();
        content += &format!(
            "{},{},{},BTC,{},{}\n",
            bisq_txid(f[0]),
            f[1],
            f[2],
            f[3],
            f[4]
        );
    }
    import_kind(conn, &content, Format::Csv).unwrap();
}
/// A Funds > Transactions export, one row per line: trade|date|details|txid byte|BTC.
fn bisq_tx(rows: &str) -> String {
    let mut csv = BISQ_TX_HEADER.to_owned();
    for line in rows.lines() {
        let f = line.trim().split('|').collect::<Vec<_>>();
        csv += &format!(
            "\"{}\",{},{},,{},{},,1\n",
            f[1],
            f[0],
            f[2],
            bisq_txid(f[3]),
            f[4]
        );
    }
    csv
}
fn import_bisq_files(
    conn: &Connection,
    trades: &str,
    transactions: Option<&str>,
) -> Result<Py, AppError> {
    import_bisq_in(conn, trades, transactions, Some("UTC"))
}
fn import_bisq_in(
    conn: &Connection,
    trades: &str,
    transactions: Option<&str>,
    timezone: Option<&str>,
) -> Result<Py, AppError> {
    let id = std::thread::current().id();
    let dir = std::env::temp_dir().join(format!("kassiber-bisq-{}-{id:?}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (file, companion) = (dir.join("tradeHistory.csv"), dir.join("transactions.csv"));
    std::fs::write(&file, trades).unwrap();
    if let Some(content) = transactions {
        std::fs::write(&companion, content).unwrap();
    }
    let request = Request {
        profile_id: "p".into(),
        wallet: String::new(),
        file: file.to_str().unwrap().into(),
        format: Format::Bisq,
        mode: None,
        companion: transactions.map(|_| companion.to_str().unwrap().into()),
        timezone: timezone.map(str::to_owned),
    };
    let result = execute(conn, request, None);
    std::fs::remove_dir_all(&dir).unwrap();
    result
}
fn import_bisq(conn: &Connection, trades: &str) -> Result<Py, AppError> {
    import_bisq_files(conn, trades, Some(BISQ_TRANSACTIONS))
}
/// Escrow legs, one per line: external id, direction, kind, msat, date.
fn bisq_legs(conn: &Connection) -> String {
    let sql = "SELECT group_concat(external_id||' '||direction||' '||kind||' '||amount||' '||occurred_at, char(10)) FROM (SELECT * FROM transactions WHERE external_id LIKE 'bisq:%' ORDER BY occurred_at,external_id)";
    conn.query_row(sql, [], |r| r.get::<_, Option<String>>(0))
        .unwrap()
        .unwrap_or_default()
}
fn bisq_count(conn: &Connection) -> i64 {
    scalar(
        conn,
        "SELECT count(*) FROM transactions WHERE external_id LIKE 'bisq:%'",
    )
}
/// A response field as compact JSON.
fn bisq_get(result: &Py, key: &str) -> String {
    let value = kassiber_envelope::get(result, key).unwrap();
    kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new()).unwrap()
}
/// `field` of each item of the response list `key`, joined by spaces.
fn bisq_items(result: &Py, key: &str, fields: &[&str]) -> Vec<String> {
    let Some(Py::List(items)) = kassiber_envelope::get(result, key) else {
        panic!("{key}");
    };
    let field = |item, f| kassiber_envelope::py_str(kassiber_envelope::get(item, f).unwrap());
    let joined = |item| {
        fields
            .iter()
            .map(|f| field(item, f))
            .collect::<Vec<_>>()
            .join(" ")
    };
    items.iter().map(joined).collect()
}
fn held(result: &Py) -> Vec<String> {
    bisq_items(result, "bisq_review_required", &["trade_id", "reason"])
}
fn bisq_transfers(conn: &Connection, op: &str, args: &[(&str, Py)]) -> Result<Py, AppError> {
    let scope = kassiber_transfers::Scope {
        workspace_id: "ws",
        profile_id: "p",
    };
    kassiber_transfers::execute(conn, &scope, op, &dict(args.to_vec()))
}
/// Pairs as `out in` external ids, sorted, and their ids in that order.
fn bisq_pairs(conn: &Connection) -> (Vec<String>, Vec<Py>) {
    let Py::List(pairs) = bisq_transfers(conn, "transfers.list", &[]).unwrap() else {
        panic!("pairs");
    };
    let external = |p: &Py, key| {
        let id = kassiber_envelope::py_str(kassiber_envelope::get(p, key).unwrap());
        let sql = "SELECT external_id FROM transactions WHERE id=?";
        conn.query_row(sql, [id], |r| r.get::<_, String>(0))
            .unwrap()
    };
    let mut named = pairs
        .iter()
        .map(|p| {
            let ends = format!(
                "{} {}",
                external(p, "out_transaction_id"),
                external(p, "in_transaction_id")
            );
            (ends, kassiber_envelope::get(p, "id").unwrap().clone())
        })
        .collect::<Vec<_>>();
    named.sort_by(|a, b| a.0.cmp(&b.0));
    named.into_iter().unzip()
}

#[test]
fn bisq_completed_trades_become_priced_escrow_ledgers_that_net_to_zero() {
    let conn = bisq_book();
    bisq_onchain(&conn, BISQ_DEPOSITS);
    let result = import_bisq(&conn, BISQ_TRADES).unwrap();
    assert_eq!(bisq_get(&result, "mode"), "\"full\"");
    assert_eq!(bisq_get(&result, "imported"), "6");
    assert_eq!(bisq_get(&result, "bisq_canceled_skipped"), "1");
    assert_eq!(held(&result), ["d15e0001 not_completed"]);
    let wallets = "SELECT count(*) FROM wallets WHERE label='Bisq' AND kind='bisq'";
    assert_eq!(scalar(&conn, wallets), 1);
    // Deposit at the deposit tx, trade one second later, payout at the
    // payout tx; each trade's legs net to exactly zero.
    assert_eq!(
        bisq_legs(&conn),
        "bisq:a1b2c3d4:deposit inbound deposit 600000000 2024-03-15T14:41:10Z
bisq:a1b2c3d4:trade inbound buy 2500000000 2024-03-15T14:41:11Z
bisq:a1b2c3d4:payout outbound withdrawal 3100000000 2024-03-16T10:00:00Z
bisq:e5f6a7b8:deposit inbound deposit 5750000000 2024-04-02T09:15:00Z
bisq:e5f6a7b8:trade outbound sell 5000000000 2024-04-02T09:15:01Z
bisq:e5f6a7b8:payout outbound withdrawal 750000000 2024-04-03T08:00:00Z"
    );
    // Only the trade leg is priced, exactly; fees stay provenance.
    let sql = "SELECT fiat_value_exact||' '||fiat_rate_exact||' '||pricing_quality||' '||pricing_timestamp||' '||raw_json FROM transactions WHERE external_id='bisq:e5f6a7b8:trade'";
    let priced: String = conn.query_row(sql, [], |r| r.get(0)).unwrap();
    let (head, raw) = priced.split_at(priced.find('{').unwrap());
    assert_eq!(head, "3100 62000.0000 exact 2024-04-02T09:05:10Z ");
    let raw: Json = serde_json::from_str(raw).unwrap();
    assert_eq!(raw["bisq_contract_time"], json!("2024-04-02T09:05:10Z"));
    assert_eq!(raw["bisq_deposit_txid"], json!(bisq_txid("c2")));
    assert_eq!(raw["bisq_fees"]["trade_fee_bsq"], json!("12.34"));
    let wrong = "SELECT count(*) FROM transactions WHERE external_id LIKE 'bisq:%' AND ((kind IN ('buy','sell'))=(pricing_quality='missing') OR fee!=0)";
    assert_eq!(scalar(&conn, wrong), 0);
    // A re-export dedupes by the stable ids.
    let result = import_bisq(&conn, BISQ_TRADES).unwrap();
    assert_eq!(bisq_get(&result, "imported"), "0");
    assert_eq!(bisq_count(&conn), 6);
}

/// The deposit pair books Bisq's stated miner fee as a network fee, never
/// suspense; later observations date the legs; the owner's unpair holds; a
/// stated-fee pair refuses revision and grouping.
#[test]
fn bisq_pairs_legs_by_txid_with_the_stated_miner_fee_and_respects_the_owner() {
    let conn = bisq_book();
    // Native observations: each deposit as the whole debit with no fee, the
    // seller's later than Bisq dated it, and both payouts.
    bisq_onchain(
        &conn,
        "a1 2024-03-15T14:45:00Z outbound 0.00605 0
c2 2024-04-02T09:20:00Z outbound 0.05754 0
b1 2024-03-16T10:00:00Z inbound 0.031 0
d2 2024-04-03T08:00:05Z inbound 0.0075 0
ee 2024-04-05T08:00:00Z outbound 0.01 0",
    );
    let result = import_bisq(&conn, BISQ_TRADES).unwrap();
    let created = bisq_items(&result, "bisq_pairs_created", &["trade_id", "leg"]);
    let legs = [
        "a1b2c3d4 deposit",
        "a1b2c3d4 payout",
        "e5f6a7b8 deposit",
        "e5f6a7b8 payout",
    ];
    assert_eq!(created, legs);
    assert_eq!(bisq_get(&result, "journal_invalidated"), "true");
    assert!(bisq_legs(&conn)
        .contains("bisq:e5f6a7b8:trade outbound sell 5000000000 2024-04-02T09:20:01Z"));
    assert!(bisq_legs(&conn)
        .contains("bisq:e5f6a7b8:payout outbound withdrawal 750000000 2024-04-03T08:00:00Z"));
    let (ends, ids) = bisq_pairs(&conn);
    let (a1, b1, c2, d2) = (
        bisq_txid("a1"),
        bisq_txid("b1"),
        bisq_txid("c2"),
        bisq_txid("d2"),
    );
    assert_eq!(
        ends,
        [
            format!("{a1} bisq:a1b2c3d4:deposit"),
            format!("bisq:a1b2c3d4:payout {b1}"),
            format!("bisq:e5f6a7b8:payout {d2}"),
            format!("{c2} bisq:e5f6a7b8:deposit"),
        ]
    );
    let sql = "SELECT group_concat(role||' '||amount_msat||' '||authored_source||' '||review_source, ', ') FROM (SELECT l.role,l.amount_msat,c.authored_source,t.review_source FROM custody_component_legs l JOIN custody_components c ON c.id=l.component_id JOIN custody_component_economic_terms t ON t.component_id=c.id JOIN custody_component_transaction_memberships m ON m.component_id=c.id JOIN transactions x ON x.id=m.transaction_id WHERE c.state='active' AND x.external_id=? ORDER BY l.role)";
    let component: String = conn.query_row(sql, [&a1], |r| r.get(0)).unwrap();
    assert_eq!(component, "destination 600000000 import provider_txid, fee 5000000 import provider_txid, source 605000000 import provider_txid");
    let suspense = "SELECT count(*) FROM custody_component_legs WHERE role='suspense'";
    assert_eq!(scalar(&conn, suspense), 0);
    // A stated-fee pair keeps its shape: revising or grouping it refuses.
    let deposit = ids[0].clone();
    let note = [("pair_id", deposit.clone()), ("notes", s("mine"))];
    let grouped = [
        ("tx_out", s(bisq_txid("ee"))),
        ("tx_in", s("bisq:a1b2c3d4:deposit")),
    ];
    for (op, args) in [("transfers.update", &note), ("transfers.pair", &grouped)] {
        let refused = bisq_transfers(&conn, op, args).unwrap_err().message;
        assert!(
            refused.contains("books the miner fee its provider stated"),
            "{refused}"
        );
    }
    // An unpaired leg is never paired again; reviewed legs are left alone.
    bisq_transfers(&conn, "transfers.unpair", &[("pair_id", deposit)]).unwrap();
    let result = import_bisq(&conn, BISQ_TRADES).unwrap();
    assert_eq!(bisq_pairs(&conn).0.len(), 3);
    let mut reasons = bisq_items(&result, "bisq_pairs_skipped", &["reason"]);
    reasons.sort();
    let expected = [
        "already_reviewed",
        "already_reviewed",
        "already_reviewed",
        "declined_by_owner",
    ];
    assert_eq!(reasons, expected);
}

#[test]
fn bisq_refuses_altcoin_markets_and_non_fiat_quotes_by_trade() {
    let conn = bisq_book();
    for (row, needle) in [
        (
            "aa11bb22,\"Mar 15, 2024 2:30:45 PM\",XMR/BTC,0.0025,0.00%,0.1,40,XMR,0.00005,0.0001,,0.006,0.006,Buy XMR,Completed",
            "Bisq trade aa11bb22 trades XMR (XMR/BTC): altcoin-market trades are not supported yet",
        ),
        (
            "cc33dd44,\"Mar 15, 2024 2:30:45 PM\",BSQ/BTC,0.00002,N/A,0.01,500,BSQ,0.00005,,1.00,N/A,N/A,Sell BSQ,Confirmed",
            "Bisq trade cc33dd44 trades BSQ (BSQ/BTC): altcoin-market trades are not supported yet",
        ),
        (
            "ee55ff66,\"Mar 15, 2024 2:30:45 PM\",BTC/USDT,60000,N/A,0.01,600,USDT,0.00005,0.0001,,0.006,0.006,Buy BTC,Completed",
            "Bisq trade ee55ff66 has market 'BTC/USDT' (currency 'USDT'), not Bitcoin against a fiat currency Kassiber knows",
        ),
    ] {
        let content = format!("{BISQ_HEADER}{}\n{row}\n", BISQ_TRADES.lines().nth(1).unwrap());
        let error = import_bisq(&conn, &content).unwrap_err();
        assert_eq!((error.code.as_str(), error.message.as_str()), ("validation", needle));
        assert!(error.hint.as_deref().unwrap().contains("generic ledger"));
        assert_eq!(bisq_count(&conn), 0);
    }
    // A canceled altcoin offer never traded, so it is skipped, not refused.
    let canceled = format!("{BISQ_HEADER}aa11bb22,\"Mar 15, 2024 2:30:45 PM\",XMR/BTC,0.0025,0.00%,0.1,40,XMR,0.00005,0.0001,,0.006,0.006,Buy XMR,Canceled\n");
    let result = import_bisq(&conn, &canceled).unwrap();
    assert_eq!(bisq_get(&result, "bisq_canceled_skipped"), "1");
}

#[test]
fn bisq_holds_unshapeable_trades_and_refuses_inexact_rows() {
    let conn = bisq_book();
    bisq_onchain(
        &conn,
        "03 2025-01-01T00:05:00Z outbound 0.006 0.0001
0d 2024-03-15T14:45:00Z outbound 0.0062 0",
    );
    let trades = "ab12cd34 Frobnicated|ab12cd35 Arbitrated|ab12cd36|ab12cd37|ab12cd38|ab12cd3a|ab12cd3b|ab12cd39";
    let trades = trades.split('|').map(|spec| {
        let (id, status) = spec.split_once(' ').unwrap_or((spec, "Completed"));
        let date = if id == "ab12cd38" { "Dec 31, 2024 11:58:00 PM" } else { "Mar 15, 2024 2:30:45 PM" };
        format!("{id},\"{date}\",BTC/EUR,60000.0000,1.00%,0.025,1500,EUR,0.00005,0.000175,,0.006,0.006,Buy BTC,{status}\n")
    });
    let transactions = bisq_tx(
        "ab12cd37-x|Mar 15, 2024 2:40:00 PM|Multisig deposit|01|-0.0061
        ab12cd37-x|Mar 16, 2024 2:40:00 PM|Multisig payout|02|0.0309
        ab12cd38-x|Jan 1, 2025 12:05:00 AM|Multisig deposit|03|-0.0061
        ab12cd38-x|Jan 2, 2025 12:05:00 AM|Multisig payout|04|0.031
        ab12cd3a-x|Mar 15, 2024 2:40:00 PM|Multisig deposit|08|-0.0061
        ab12cd3a-x|Mar 16, 2024 2:40:00 PM|Multisig payout|09|0.031
        ab12cd3b-x|Mar 15, 2024 2:40:00 PM|Multisig deposit|0d|-0.0061
        ab12cd3b-x|Mar 16, 2024 2:40:00 PM|Multisig payout|0e|0.031
        ab12cd39-x|Mar 15, 2024 2:41:10 PM|Multisig deposit|05|-0.00605",
    );
    let trades = BISQ_HEADER.to_owned() + &trades.collect::<String>();
    let result = import_bisq_files(&conn, &trades, Some(&transactions)).unwrap();
    assert_eq!(bisq_get(&result, "imported"), "0");
    // Unknown and arbitrated states; no escrow rows; a payout that is not
    // the contract's; a deposit in the next tax year; an unobserved deposit;
    // an observed debit that is not Bisq's; a missing payout.
    let reasons = [
        "ab12cd34 not_completed",
        "ab12cd35 not_completed",
        "ab12cd36 missing_deposit_tx",
        "ab12cd37 payout_amount_mismatch",
        "ab12cd38 tax_year_boundary",
        "ab12cd3a deposit_not_in_book",
        "ab12cd3b deposit_debit_mismatch",
        "ab12cd39 missing_payout_tx",
    ];
    assert_eq!(held(&result), reasons);
    // A completed trade without exact deposits, amount, price, side or date
    // is refused by trade id; nothing is written for its file.
    for (row, message) in [
        ("ab12cd36,\"Mar 15, 2024 2:30:45 PM\",BTC/EUR,60000.0000,1.00%,0.025,1500,EUR,0.00005,0.000175,,N/A,0.006,Buy BTC,Completed", "Bisq trade ab12cd36 has no exact buyer security deposit ('N/A')"),
        ("ab12cd37,\"Mar 15, 2024 2:30:45 PM\",BTC/EUR,60000.0000,1.00%,0,1500,EUR,0.00005,0.000175,,0.006,0.006,Buy BTC,Completed", "Bisq trade ab12cd37 has no positive BTC amount"),
        ("ab12cd38,\"Mar 15, 2024 2:30:45 PM\",BTC/EUR,,1.00%,0.025,1500,EUR,0.00005,0.000175,,0.006,0.006,Buy BTC,Completed", "Bisq trade ab12cd38 has no exact price ('')"),
        ("ab12cd39,\"Mar 15, 2024 2:30:45 PM\",BTC/EUR,60000.0000,1.00%,0.025,1500,EUR,0.00005,0.000175,,0.006,0.006,Swap BTC,Completed", "Bisq trade ab12cd39 has an unrecognized offer type 'Swap BTC'"),
        ("ab12cd40,15/03/2024 14:30:45,BTC/EUR,60000.0000,1.00%,0.025,1500,EUR,0.00005,0.000175,,0.006,0.006,Buy BTC,Completed", "Bisq trade ab12cd40 has an unreadable date '15/03/2024 14:30:45'"),
    ] {
        let error = import_bisq(&conn, &format!("{BISQ_TRADES}{row}\n")).unwrap_err();
        assert_eq!(error.message, message);
        assert_eq!(bisq_count(&conn), 0);
    }
    let error = import_bisq(&conn, "Trade ID,Date/Time\nx,y\n").unwrap_err();
    let columns = "Bisq trade history CSV is missing required columns: Market, Price";
    assert!(error.message.starts_with(columns), "{}", error.message);
    let error = import_bisq_files(&conn, BISQ_TRADES, None).unwrap_err();
    assert_eq!(
        error.message,
        "A Bisq import needs the Funds > Transactions export as well"
    );
    let broken = format!("{BISQ_TX_HEADER}\"Mar 15, 2024 2:41:10 PM\",a1b2c3d4-1,Multisig deposit,,not-a-txid,-0.00605,,1\n");
    let error = import_bisq_files(&conn, BISQ_TRADES, Some(&broken)).unwrap_err();
    assert!(error
        .message
        .contains("unreadable transaction id 'not-a-txid'"));
    assert_eq!(bisq_count(&conn), 0);
}

#[test]
fn bisq_reads_german_exports_and_zero_deposits() {
    let conn = bisq_book();
    bisq_onchain(&conn, "aa 2024-03-15T14:40:00Z outbound 0 0.0001");
    let trades = "Handels-ID,Datum/Zeit,Markt,Preis,Abweichung,Betrag in BTC,Betrag,Währung,Transaktionsgebühr,Handelsgebühr BTC,Handelsgebühr BSQ,Käufer-Kaution,Verkäufer-Kaution,Angebotstyp,Status\n\
        0a0b0c0d,15.03.2024 14:30:45,BTC/EUR,60000.0000,1.00%,0.01,600,EUR,0.00005,0.00007,,0,0.006,BTC kaufen,Abgeschlossen\n\
        0a0b0c0e,16.03.2024 08:00:00,BTC/EUR,60000.0000,1.00%,0.01,600,EUR,0.00005,0.00007,,0.006,0.006,BTC verkaufen,Abgebrochen\n";
    let (aa, bb) = (bisq_txid("aa"), bisq_txid("bb"));
    let transactions = format!(
        "Datum/Zeit,Handels-ID,Details,Adresse,Transaktions-ID,Betrag in BTC,Notiz,Bestätigungen\n\
        15.03.2024 14:40:00,0a0b0c0d-1,Multisig deposit,,{aa},-0.0001,,1\n\
        16.03.2024 09:00:00,0a0b0c0d-1,Multisig payout,,{bb},0.01,,1\n"
    );
    let result = import_bisq_files(&conn, trades, Some(&transactions)).unwrap();
    assert_eq!(bisq_get(&result, "bisq_canceled_skipped"), "1");
    // A zero buyer deposit writes no deposit leg.
    assert_eq!(
        bisq_legs(&conn),
        "bisq:0a0b0c0d:trade inbound buy 1000000000 2024-03-15T14:40:01Z
bisq:0a0b0c0d:payout outbound withdrawal 1000000000 2024-03-16T09:00:00Z"
    );
}

/// Every hold as `trade reason state txids`.
fn bisq_hold_rows(conn: &Connection) -> String {
    let sql = "SELECT group_concat(trade_id||' '||reason||' '||state||' '||txids_json, char(10)) FROM (SELECT * FROM provider_trade_holds ORDER BY trade_id)";
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// The first hold creates the opt-in table; holds quarantine the
/// transactions they name, invalidate journals when written, keep every txid
/// any export named, and clear only when a completed trade explains them all.
#[test]
fn bisq_held_trades_persist_quarantine_and_clear_only_on_proof() {
    let f4_deposit = BISQ_TRANSACTIONS
        .lines()
        .find(|l| l.contains("f4f4"))
        .unwrap()
        .to_owned()
        + "\n";
    let stray = f4_deposit
        .replace("f4f4", "f6f6")
        .replace("11:20:00", "11:21:00");
    let payout = bisq_tx("d15e0001-2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e-199|May 3, 2024 9:00:00 AM|Multisig payout|f5|0.026");
    let payout = payout.trim_start_matches(BISQ_TX_HEADER);
    let completed = BISQ_TRADES.replace(",Buy BTC,Mediated", ",Buy BTC,Completed");
    let only_first = BISQ_TRADES.lines().take(2).collect::<Vec<_>>().join("\n") + "\n";
    let held_only = format!("{BISQ_HEADER}{}\n", BISQ_TRADES.lines().last().unwrap());
    let (f4, f6) = (bisq_txid("f4"), bisq_txid("f6"));
    for with_stray in [false, true] {
        let conn = bisq_book();
        bisq_onchain(
            &conn,
            "a1 2024-03-15T14:41:10Z outbound 0.00605 0
c2 2024-04-02T09:15:00Z outbound 0.05754 0
f4 2024-05-01T11:25:00Z outbound 0.00605 0",
        );
        let table = "SELECT count(*) FROM sqlite_master WHERE name='provider_trade_holds'";
        import_bisq(&conn, &only_first).unwrap();
        assert_eq!(scalar(&conn, table), 0);
        // Only the held trade: the hold alone writes and invalidates.
        let version = "SELECT journal_input_version FROM profiles WHERE id='p'";
        let before = scalar(&conn, version);
        let result = import_bisq(&conn, &held_only).unwrap();
        assert_eq!(bisq_get(&result, "journal_invalidated"), "true");
        assert!(scalar(&conn, version) > before);
        assert_eq!(scalar(&conn, table), 1);
        assert_eq!(
            bisq_hold_rows(&conn),
            format!("d15e0001 not_completed held [\"{f4}\"]")
        );
        let quarantines = bisq_hold_quarantines(&conn, "p").unwrap();
        let deposit_sql = "SELECT id FROM transactions WHERE external_id=?";
        let deposit: String = conn.query_row(deposit_sql, [&f4], |r| r.get(0)).unwrap();
        assert_eq!(quarantines.len(), 1);
        let quarantine =
            kassiber_pyjson::dumps(&quarantines[0], &kassiber_pyjson::Options::new()).unwrap();
        assert!(quarantine.contains(&format!("\"transaction_id\": \"{deposit}\"")));
        assert!(quarantine.contains(&format!("\"reason\": \"{BISQ_HOLD_REASON}\"")));
        assert!(
            quarantine.contains(r#"\"hold_reason\": \"not_completed\""#),
            "{quarantine}"
        );
        // An unchanged hold writes nothing; a less complete export or one
        // that leaves the trade out never drops a txid.
        let result = import_bisq(&conn, &held_only).unwrap();
        assert_eq!(bisq_get(&result, "journal_invalidated"), "false");
        let without_f4 = BISQ_TRANSACTIONS.replace(&f4_deposit, "");
        import_bisq_files(&conn, BISQ_TRADES, Some(&without_f4)).unwrap();
        import_bisq(&conn, &only_first).unwrap();
        assert!(bisq_hold_rows(&conn).ends_with(&format!("held [\"{f4}\"]")));
        let mut transactions = BISQ_TRANSACTIONS.to_owned();
        if with_stray {
            import_bisq_files(&conn, BISQ_TRADES, Some(&(transactions.clone() + &stray))).unwrap();
            assert!(bisq_hold_rows(&conn).ends_with(&format!("[\"{f4}\", \"{f6}\"]")));
        }
        transactions += payout;
        let result = import_bisq_files(&conn, &completed, Some(&transactions)).unwrap();
        if with_stray {
            // The completed export does not explain the second deposit.
            assert_eq!(held(&result), ["d15e0001 conflicting_escrow_txs"]);
            assert!(!bisq_legs(&conn).contains("bisq:d15e0001:"));
            // Excluding the row (the owner's decision) ends its quarantine.
            conn.execute("UPDATE transactions SET excluded=1 WHERE id=?", [&deposit])
                .unwrap();
            assert!(bisq_hold_quarantines(&conn, "p").unwrap().is_empty());
        } else {
            assert!(held(&result).is_empty(), "{result:?}");
            assert!(bisq_hold_rows(&conn).starts_with("d15e0001 not_completed cleared"));
            assert!(bisq_hold_quarantines(&conn, "p").unwrap().is_empty());
            // Rolling the completing import back (imports rollback deletes
            // its legs) leaves no settlement evidence: the hold is open again
            // once the pair's component, now one-sided (it blocks reports
            // until repaired), is superseded too.
            let legs = "DELETE FROM transactions WHERE external_id LIKE 'bisq:d15e0001:%'";
            assert_eq!(conn.execute(legs, []).unwrap(), 3);
            let pair = "UPDATE custody_components SET state='superseded' WHERE id IN (SELECT component_id FROM custody_component_transaction_memberships WHERE transaction_id=?)";
            assert_eq!(conn.execute(pair, [&deposit]).unwrap(), 1);
            let reopened = bisq_hold_quarantines(&conn, "p").unwrap();
            assert_eq!(reopened.len(), 1);
            assert_eq!(
                kassiber_envelope::get(&reopened[0], "transaction_id"),
                Some(&Py::Str(deposit.clone().into()))
            );
            // Importing the completed export again settles it once more.
            let result = import_bisq_files(&conn, &completed, Some(&transactions)).unwrap();
            assert!(held(&result).is_empty(), "{result:?}");
            assert!(bisq_hold_quarantines(&conn, "p").unwrap().is_empty());
        }
    }
}

/// Bisq writes local time; the import reads it in the named zone, exactly,
/// and refuses a wall time the zone skipped or showed twice.
#[test]
fn bisq_reads_local_time_in_the_named_zone() {
    let trades = format!(
        "{BISQ_HEADER}0b0c0d0e,\"Dec 31, 2024 11:10:00 PM\",BTC/EUR,90000.0000,1.00%,0.01,900,EUR,0.00005,0.00007,,0.006,0.006,Buy BTC,Completed\n"
    );
    let transactions = bisq_tx(
        "0b0c0d0e-1|Dec 31, 2024 11:20:00 PM|Multisig deposit|0b|-0.0061
        0b0c0d0e-1|Jan 2, 2025 9:00:00 AM|Multisig payout|0c|0.016",
    );
    let conn = bisq_book();
    bisq_onchain(&conn, "0b 2024-12-31T22:20:00Z outbound 0.0061 0");
    let error = import_bisq_in(&conn, &trades, Some(&transactions), None).unwrap_err();
    assert!(error.hint.unwrap().contains("--timezone"));
    let mars = import_bisq_in(&conn, &trades, Some(&transactions), Some("Mars/Olympus"));
    assert_eq!(
        mars.unwrap_err().message,
        "Unknown time zone 'Mars/Olympus'"
    );
    // Vienna's New Year's Eve 23:10 is 22:10 UTC, still in the old year.
    let vienna = Some("Europe/Vienna");
    let result = import_bisq_in(&conn, &trades, Some(&transactions), vienna).unwrap();
    assert!(held(&result).is_empty());
    assert_eq!(
        bisq_legs(&conn),
        "bisq:0b0c0d0e:deposit inbound deposit 600000000 2024-12-31T22:20:00Z
bisq:0b0c0d0e:trade inbound buy 1000000000 2024-12-31T22:20:01Z
bisq:0b0c0d0e:payout outbound withdrawal 1600000000 2025-01-02T08:00:00Z"
    );
    let sql = "SELECT raw_json FROM transactions WHERE external_id='bisq:0b0c0d0e:trade'";
    let raw: String = conn.query_row(sql, [], |r| r.get(0)).unwrap();
    let raw: Json = serde_json::from_str(&raw).unwrap();
    assert_eq!(raw["bisq_contract_time"], json!("2024-12-31T22:10:00Z"));
    assert_eq!(raw["bisq_timezone"], json!("Europe/Vienna"));
    for (date, needle) in [
        ("Oct 27, 2024 2:30:00 AM", "showed twice"),
        ("Mar 31, 2024 2:30:00 AM", "skipped"),
    ] {
        let shifted = trades.replace("Dec 31, 2024 11:10:00 PM", date);
        let error =
            import_bisq_in(&bisq_book(), &shifted, Some(&transactions), vienna).unwrap_err();
        assert!(error.message.contains(needle), "{}", error.message);
    }
}

/// An export that contradicts legs already in the book refuses atomically,
/// naming the trade: other amounts, moved escrow dates, another time zone,
/// a stored trade it would hold, and short-id collisions.
#[test]
fn bisq_refuses_exports_that_contradict_stored_legs() {
    let conn = bisq_book();
    bisq_onchain(&conn, BISQ_DEPOSITS);
    import_bisq(&conn, BISQ_TRADES).unwrap();
    let before = scalar(&conn, "SELECT count(*) FROM transactions");
    let refuses = |result: Result<Py, AppError>, trade: &str| {
        let error = result.unwrap_err();
        let prefix =
            format!("Bisq trade {trade} conflicts with its escrow legs already in the book");
        assert!(error.message.starts_with(&prefix), "{}", error.message);
        assert!(error.hint.unwrap().contains("imports rollback"));
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), before);
    };
    let sell = "e5f6a7b8,\"Apr 2, 2024 9:05:10 AM\",BTC/EUR,62000.0000,-0.50%,0.05,3100";
    let resized = BISQ_TRADES.replace(sell, &sell.replace("0.05,3100", "0.06,3720"));
    refuses(import_bisq(&conn, &resized), "e5f6a7b8");
    let mediated = BISQ_TRADES.replace("Sell BTC,Completed", "Sell BTC,Mediated");
    refuses(import_bisq(&conn, &mediated), "e5f6a7b8");
    let london = Some("Europe/London");
    refuses(
        import_bisq_in(&conn, BISQ_TRADES, Some(BISQ_TRANSACTIONS), london),
        "a1b2c3d4",
    );
    // A later native confirmation of the buyer's deposit moves its leg.
    let moved = "UPDATE transactions SET occurred_at='2024-03-15T15:30:00Z' WHERE external_id=?";
    conn.execute(moved, [bisq_txid("a1")]).unwrap();
    refuses(import_bisq(&conn, BISQ_TRADES), "a1b2c3d4");
    // Two trades whose full ids share a short id refuse as well.
    let other = bisq_tx("e5f6a7b8-ffffffff|Apr 2, 2024 9:15:00 AM|Multisig payout|ee|0.0075");
    let collision = BISQ_TRANSACTIONS.to_owned() + other.trim_start_matches(BISQ_TX_HEADER);
    let error = import_bisq_files(&conn, BISQ_TRADES, Some(&collision)).unwrap_err();
    assert!(
        error.message.contains("names two different Bisq trades"),
        "{}",
        error.message
    );
}

/// Escrow evidence that does not name one fact is held or refused, never
/// read from whichever row came first: several owned observations of one
/// escrow transaction, Funds rows that disagree about one txid, a re-export
/// that moves a trade to another escrow transaction, and two trades naming
/// the same escrow transaction.
#[test]
fn bisq_holds_or_refuses_ambiguous_escrow_evidence() {
    // The buyer's deposit observed in two owned wallets.
    let conn = bisq_book();
    bisq_onchain(&conn, BISQ_DEPOSITS);
    conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('second','ws','p','Second','custom','2025-01-01')").unwrap();
    let copy = "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at) SELECT 'copy',workspace_id,profile_id,'second','fp-copy',external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at FROM transactions WHERE external_id=?";
    conn.execute(copy, [bisq_txid("a1")]).unwrap();
    let result = import_bisq(&conn, BISQ_TRADES).unwrap();
    assert_eq!(
        held(&result),
        ["a1b2c3d4 deposit_debit_mismatch", "d15e0001 not_completed"]
    );
    assert!(!bisq_legs(&conn).contains("a1b2c3d4"));

    // A second Funds row for the buyer's payout txid with another amount
    // holds the trade in either order; an identical second row does not.
    let payout = "\"Mar 16, 2024 10:00:00 AM\",a1b2c3d4-0f1e2d3c-4b5a-4978-8a9b-0c1d2e3f4a5b-199,Multisig payout,Received with: bc1qsyntheticpayout00000000000000000000000,b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1,0.031,,3100\n";
    let other = payout.replace(",0.031,", ",0.0311,");
    let (header, body) = BISQ_TRANSACTIONS.split_at(BISQ_TX_HEADER.len());
    for transactions in [
        format!("{BISQ_TRANSACTIONS}{other}"),
        format!("{header}{other}{body}"),
    ] {
        let conn = bisq_book();
        bisq_onchain(&conn, BISQ_DEPOSITS);
        let result = import_bisq_files(&conn, BISQ_TRADES, Some(&transactions)).unwrap();
        assert_eq!(
            held(&result),
            ["a1b2c3d4 conflicting_escrow_txs", "d15e0001 not_completed"]
        );
    }
    let conn = bisq_book();
    bisq_onchain(&conn, BISQ_DEPOSITS);
    let doubled = format!("{BISQ_TRANSACTIONS}{payout}");
    let result = import_bisq_files(&conn, BISQ_TRADES, Some(&doubled)).unwrap();
    assert_eq!(held(&result), ["d15e0001 not_completed"]);

    // A re-export naming another payout txid for a stored trade refuses.
    let before = scalar(&conn, "SELECT count(*) FROM transactions");
    let moved = BISQ_TRANSACTIONS.replace(&bisq_txid("d2"), &bisq_txid("dd"));
    let error = import_bisq_files(&conn, BISQ_TRADES, Some(&moved)).unwrap_err();
    assert!(
        error
            .message
            .starts_with("Bisq trade e5f6a7b8 conflicts with its escrow legs already in the book"),
        "{}",
        error.message
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), before);

    // Another trade naming a stored trade's escrow transactions refuses, and
    // so do two trades of one export naming the same ones.
    let twin = "a1b2c3d5,\"Mar 15, 2024 2:30:45 PM\",BTC/EUR,60000.0000,1.00%,0.025,1500,EUR,0.00005,0.000175,,0.006,0.006,Buy BTC,Completed\n";
    let twin_rows = bisq_tx(
        "a1b2c3d5-1|Mar 15, 2024 2:41:10 PM|Multisig deposit|a1|-0.00605
        a1b2c3d5-1|Mar 16, 2024 10:00:00 AM|Multisig payout|b1|0.031",
    );
    let twin_rows = twin_rows.trim_start_matches(BISQ_TX_HEADER);
    let error = import_bisq_files(
        &conn,
        &format!("{BISQ_HEADER}{twin}"),
        Some(&format!("{BISQ_TRANSACTIONS}{twin_rows}")),
    )
    .unwrap_err();
    let claim = format!(
        "Bisq trade a1b2c3d5 names escrow transaction {}, which trade a1b2c3d4 also names",
        bisq_txid("a1")
    );
    assert_eq!(error.message, claim);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), before);
    let conn = bisq_book();
    bisq_onchain(&conn, BISQ_DEPOSITS);
    let error = import_bisq_files(
        &conn,
        &format!("{BISQ_TRADES}{twin}"),
        Some(&format!("{BISQ_TRANSACTIONS}{twin_rows}")),
    )
    .unwrap_err();
    assert_eq!(error.message, claim);
    assert_eq!(bisq_count(&conn), 0);
}

/// Two imports into different Bisq wallets of one book never both book a
/// trade: the export is checked against the book under the write lock, so an
/// import that waited for another's commit sees its legs and refuses.
#[test]
fn bisq_checks_the_book_under_the_write_lock() {
    let dir = std::env::temp_dir().join(format!(
        "kassiber-bisq-lock-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("book.sqlite3");
    let open = |path: &std::path::Path| {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=20000",
        )
        .unwrap();
        conn
    };
    let first = open(&path);
    let creation: Json =
        serde_json::from_str(include_str!("../../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        first.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    first.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','ws','Book','EUR','2025-01-01');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet','ws','p','Source','custom','{}','2025-01-01'),('second-bisq','ws','p','Bisq Two','bisq','{}','2025-01-01');
        INSERT OR IGNORE INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','2025-01-01')").unwrap();
    bisq_onchain(&first, BISQ_DEPOSITS);
    // The first import holds the write lock until it commits.
    first.execute_batch("BEGIN IMMEDIATE").unwrap();
    import_bisq(&first, BISQ_TRADES).unwrap();
    let files = dir.clone();
    let second = std::thread::spawn(move || {
        let conn = open(&files.join("book.sqlite3"));
        let (trades, companion) = (files.join("history.csv"), files.join("transactions.csv"));
        std::fs::write(&trades, BISQ_TRADES).unwrap();
        std::fs::write(&companion, BISQ_TRANSACTIONS).unwrap();
        execute(
            &conn,
            Request {
                profile_id: "p".into(),
                wallet: "Bisq Two".into(),
                file: trades.to_str().unwrap().into(),
                format: Format::Bisq,
                mode: None,
                companion: Some(companion.to_str().unwrap().into()),
                timezone: Some("UTC".into()),
            },
            None,
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    first.execute_batch("COMMIT").unwrap();
    let error = second.join().unwrap().unwrap_err();
    assert!(
        error
            .message
            .contains("conflicts with its escrow legs already in the book"),
        "{}",
        error.message
    );
    assert_eq!(
        scalar(
            &first,
            "SELECT count(*) FROM transactions WHERE wallet_id='second-bisq'"
        ),
        0
    );
    drop(first);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The escrow ledger needs a dedicated bisq wallet; a wallet of another kind
/// is refused before anything is written.
#[test]
fn bisq_imports_only_into_a_bisq_wallet() {
    let conn = bisq_book();
    bisq_onchain(&conn, BISQ_DEPOSITS);
    let before = scalar(&conn, "SELECT count(*) FROM transactions");
    let dir = std::env::temp_dir().join(format!("kassiber-bisq-kind-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (trades, transactions) = (dir.join("t.csv"), dir.join("x.csv"));
    std::fs::write(&trades, BISQ_TRADES).unwrap();
    std::fs::write(&transactions, BISQ_TRANSACTIONS).unwrap();
    let request = |wallet: &str| Request {
        profile_id: "p".into(),
        wallet: wallet.into(),
        file: trades.to_str().unwrap().into(),
        format: Format::Bisq,
        mode: None,
        companion: Some(transactions.to_str().unwrap().into()),
        timezone: Some("UTC".into()),
    };
    let error = execute(&conn, request("Source"), None).unwrap_err();
    let refusal = "Bisq trades import into a bisq wallet; 'Source' is a custom wallet";
    assert_eq!(error.message, refusal);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), before);
    conn.execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('mine','ws','p','My escrow','bisq','{}','2025-01-01')").unwrap();
    let result = execute(&conn, request("My escrow"), None).unwrap();
    assert_eq!(bisq_get(&result, "wallet"), "\"My escrow\"");
    std::fs::remove_dir_all(dir).unwrap();
}

/// docs/plan/24-crypto-assets.md: a provider-reported swap is paired once,
/// with the country's policy, and other assets are stored exactly or refused.
#[test]
fn provider_swaps_pair_once_and_other_assets_store_exactly() {
    let conn = fixture();
    conn.execute_batch("UPDATE profiles SET tax_country='at', fiat_currency='EUR' WHERE id='p'")
        .unwrap();
    assets::Scope::Crypto.store(&conn, "p", "wallet").unwrap();
    let leg = |txid: &str, direction: &str, asset: &str, amount: &str, swap: Option<&str>| {
        let mut row = json!({"txid": txid, "occurred_at": "2025-04-01T10:00:00Z", "direction": direction,
            "asset": asset, "amount": amount, "kind": "swap", "fiat_value": "4000", "fiat_currency": "EUR"});
        if let Some(swap) = swap {
            row["swap_ref"] = json!(swap);
        }
        row
    };
    let rows = json!([
        {"txid": "b", "occurred_at": "2025-01-02T10:00:00Z", "direction": "inbound", "asset": "BTC",
         "amount": "0.1", "kind": "buy", "fiat_value": "9000", "fiat_currency": "EUR"},
        leg("s-out", "outbound", "BTC", "0.05", Some("trade-1")),
        leg("s-in", "inbound", "ETH", "1.12345678901", Some("trade-1")),
    ]);
    let first = import_kind(&conn, &rows.to_string(), Format::Json).unwrap();
    assert_eq!(
        kassiber_envelope::get(&first, "swap_pairs"),
        Some(&Py::from(1))
    );
    // 11 decimals are stored exactly, in the same unit as msat.
    assert_eq!(
        scalar(
            &conn,
            "SELECT amount FROM transactions WHERE external_id='s-in'"
        ),
        112_345_678_901
    );
    let scope = kassiber_transfers::Scope {
        workspace_id: "ws",
        profile_id: "p",
    };
    let pairs = kassiber_transfers::execute(
        &conn,
        &scope,
        "transfers.list",
        &dict(Vec::<(&str, Py)>::new()),
    )
    .unwrap();
    let listed = kassiber_pyjson::dumps(&pairs, &Default::default()).unwrap();
    assert!(listed.contains(r#""policy": "carrying-value""#), "{listed}");
    assert!(listed.contains(r#""pair_source": "provider""#), "{listed}");
    // A re-run keeps the pair and authors no second one.
    let again = import_kind(&conn, &rows.to_string(), Format::Json).unwrap();
    assert_eq!(
        kassiber_envelope::get(&again, "swap_pairs"),
        Some(&Py::from(0))
    );
    // An import without swaps reports none.
    let plain = json!([{"txid": "c", "occurred_at": "2025-05-01T10:00:00Z", "direction": "inbound",
        "asset": "BTC", "amount": "0.01", "kind": "deposit"}]);
    let plain = import_kind(&conn, &plain.to_string(), Format::Json).unwrap();
    assert_eq!(kassiber_envelope::get(&plain, "swap_pairs"), None);

    // Finer than 11 decimals, or too large: refused, never rounded.
    for amount in ["0.000000000001", "100000000"] {
        let finer = json!([leg("x", "inbound", "ETH", amount, None)]);
        let error = import_kind(&conn, &finer.to_string(), Format::Json).unwrap_err();
        assert_eq!(error.code, "validation", "{amount}: {error:?}");
    }
    // A swap needs exactly one leg each way.
    let lopsided = json!([
        leg("y-1", "outbound", "BTC", "0.01", Some("trade-2")),
        leg("y-2", "outbound", "BTC", "0.01", Some("trade-2")),
    ]);
    let error = import_kind(&conn, &lopsided.to_string(), Format::Json).unwrap_err();
    assert_eq!(error.code, "protocol_error", "{error:?}");
    assert_eq!(
        scalar(
            &conn,
            "SELECT COUNT(*) FROM transactions WHERE external_id LIKE 'y-%'"
        ),
        0
    );
}

/// A provider swap whose leg is already paired elsewhere is refused and the
/// whole import rolls back; only the exact pair already present is a re-run.
#[test]
fn provider_swap_with_a_leg_paired_elsewhere_is_refused_and_the_exact_pair_is_kept() {
    let conn = fixture();
    conn.execute_batch("UPDATE profiles SET tax_country='at', fiat_currency='EUR' WHERE id='p'")
        .unwrap();
    assets::Scope::Crypto.store(&conn, "p", "wallet").unwrap();
    let leg = |txid: &str, direction: &str, asset: &str, swap: Option<&str>| {
        let mut row = json!({"txid": txid, "occurred_at": "2025-04-01T10:00:00Z", "direction": direction,
            "asset": asset, "amount": "0.05", "kind": "swap", "fiat_value": "4000", "fiat_currency": "EUR"});
        if let Some(swap) = swap {
            row["swap_ref"] = json!(swap);
        }
        row
    };
    let id = |external: &str| -> String {
        conn.query_row(
            "SELECT id FROM transactions WHERE external_id=?",
            [external],
            |r| r.get(0),
        )
        .unwrap()
    };
    let scope = kassiber_transfers::Scope {
        workspace_id: "ws",
        profile_id: "p",
    };
    let pair = |out: &str, into: &str| {
        kassiber_transfers::execute(
            &conn,
            &scope,
            "transfers.pair",
            &dict([("tx_out", s(id(out))), ("tx_in", s(id(into)))]),
        )
        .unwrap();
    };
    // The outbound leg is already paired with another inbound by hand.
    let first = json!([
        leg("s-out", "outbound", "BTC", None),
        leg("other-in", "inbound", "BTC", None),
    ]);
    import_kind(&conn, &first.to_string(), Format::Json).unwrap();
    pair("s-out", "other-in");
    let transactions = scalar(&conn, "SELECT count(*) FROM transactions");
    let swap = json!([
        leg("s-out", "outbound", "BTC", Some("trade-1")),
        leg("s-in", "inbound", "ETH", Some("trade-1")),
    ]);
    let error = import_kind(&conn, &swap.to_string(), Format::Json).unwrap_err();
    assert_eq!(error.code, "conflict", "{error:?}");
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM transactions"),
        transactions
    );
    assert_eq!(
        kassiber_transfers::transaction_pairs(&conn, &scope).unwrap(),
        vec![(id("s-out"), id("other-in"))]
    );

    // The exact pair, already authored, is kept and not authored again.
    let exact = json!([
        leg("t-out", "outbound", "BTC", None),
        leg("t-in", "inbound", "ETH", None),
    ]);
    import_kind(&conn, &exact.to_string(), Format::Json).unwrap();
    pair("t-out", "t-in");
    let swap = json!([
        leg("t-out", "outbound", "BTC", Some("trade-2")),
        leg("t-in", "inbound", "ETH", Some("trade-2")),
    ]);
    let again = import_kind(&conn, &swap.to_string(), Format::Json).unwrap();
    assert_eq!(
        kassiber_envelope::get(&again, "swap_pairs"),
        Some(&Py::from(0))
    );
    assert_eq!(
        kassiber_transfers::transaction_pairs(&conn, &scope)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn asset_scope_is_validated_and_defaults_to_bitcoin() {
    let conn = fixture();
    assert_eq!(
        assets::Scope::of_wallet(&conn, "wallet").unwrap(),
        assets::Scope::Bitcoin
    );
    assets::Scope::Crypto.store(&conn, "p", "wallet").unwrap();
    assert_eq!(
        assets::Scope::of_wallet(&conn, "wallet").unwrap(),
        assets::Scope::Crypto
    );
    let error = lifecycle::execute(
        &conn,
        lifecycle::Operation::Update {
            profile_id: "p".into(),
            wallet: "wallet".into(),
            label: None,
            account: None,
            config: json!({"asset_scope": "stocks"}),
            clear: vec![],
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "validation");
    let mut tally = assets::Tally::default();
    assert!(tally.admit(assets::Scope::Bitcoin, "BTC", assets::Class::Bitcoin));
    assert!(!tally.admit(assets::Scope::Bitcoin, "ETH", assets::Class::Crypto));
    assert!(!tally.admit(assets::Scope::Bitcoin, "ETH", assets::Class::Crypto));
    assert!(!tally.admit(assets::Scope::Crypto, "AAPL", assets::Class::Excluded));
    assert!(!tally.admit(assets::Scope::Bitcoin, "EUR", assets::Class::Fiat));
    assert!(tally.admit(assets::Scope::Crypto, "SOL", assets::Class::Crypto));
    assert_eq!(
        kassiber_pyjson::dumps(
            &tally.envelope(assets::Scope::Bitcoin, "Tom's Ex"),
            &Default::default()
        )
        .unwrap(),
        r#"{"scope": "bitcoin", "skipped": {"ETH": 2}, "excluded": {"AAPL": 1}, "next_step": "kassiber wallets update --wallet 'Tom'\\''s Ex' --config '{\"asset_scope\": \"crypto\"}', then sync or import again to keep ETH"}"#
    );
}

mod supplemental_assets;
