use kassiber_envelope::{get, s};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::params, Book};
use kassiber_transactions::{
    metadata::{execute, Bip329, Command},
    Scope,
};

fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}
fn book() -> (tempfile::TempDir, Book) {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    book.connection().execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025');INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('a','w','p','Alpha','address','{\"addresses\":[\"1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa\"]}','2025'),('b','w','p','Beta','address','{}','2025');").unwrap();
    for (id, wallet, external, direction) in [
        ("a1", "a", "same", "inbound"),
        ("a2", "a", "same", "inbound"),
        ("b1", "b", "single", "outbound"),
    ] {
        book.connection().execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,kind,created_at) VALUES(?,'w','p',?,?,?,'BTC',?,123000,1000,'2025-01-01','deposit',?)",params![id,wallet,id,external,direction,id]).unwrap();
    }
    (tmp, book)
}
fn run(book: &Book, command: Bip329) -> Value {
    execute(book.connection(), &scope(), &Command::Bip329(command)).unwrap()
}
fn write(tmp: &tempfile::TempDir, body: &str) -> String {
    let path = tmp.path().join("labels.jsonl");
    std::fs::write(&path, body).unwrap();
    path.to_str().unwrap().into()
}
fn import(book: &Book, file: &str, ambiguous: bool) -> Value {
    run(
        book,
        Bip329::Import {
            file: file.into(),
            apply_ambiguous: ambiguous,
            source: "gui".into(),
        },
    )
}
fn scalar(book: &Book, query: &str) -> i64 {
    book.connection()
        .query_row(query, [], |r| r.get(0))
        .unwrap()
}
fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(rows) => rows,
        _ => panic!("list: {v:?}"),
    }
}
fn field<'a>(v: &'a Value, k: &str) -> &'a Value {
    get(v, k).unwrap()
}
fn export(
    tmp: &tempfile::TempDir,
    book: &Book,
    mode: &str,
    wallets: Option<Vec<&str>>,
) -> (Value, String) {
    let path = tmp.path().join("export.jsonl");
    let result = run(
        book,
        Bip329::Export {
            file: path.to_str().unwrap().into(),
            wallets: wallets.map(|v| v.into_iter().map(str::to_owned).collect()),
            mode: mode.into(),
        },
    );
    (result, std::fs::read_to_string(path).unwrap())
}

#[test]
fn duplicate_merge_keeps_null_fields_and_nested_kassiber_data_and_projects_only_last_tag() {
    let (tmp, book) = book();
    let file = write(
        &tmp,
        r#"{"type":"tx","ref":"single","label":"first","origin":"old","first":{"z":2},"kassiber":{"keep":1}}
{"type":"tx","ref":"single","label":"last","origin":null,"second":true,"kassiber":"opaque"}
{"type":"output","ref":"out:0","label":"locked","spendable":false}
{"type":"output","ref":"out:0","label":null,"spendable":null}
"#,
    );
    let result = import(&book, &file, false);
    assert_eq!(field(&result, "imported"), &Value::Int(2.into()));
    assert_eq!(field(&result, "updated"), &Value::Int(2.into()));
    assert_eq!(
        field(&result, "transaction_tags_added"),
        &Value::Int(1.into())
    );
    let label: String = book
        .connection()
        .query_row("SELECT label FROM tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(label, "last");
    let data: String = book
        .connection()
        .query_row(
            "SELECT data_json FROM bip329_labels WHERE record_type='tx'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let data = kassiber_pytext::loads(&data).unwrap();
    assert_eq!(
        field(&data, "first"),
        &kassiber_envelope::dict([("z", Value::Int(2.into()))])
    );
    assert_eq!(
        field(field(&data, "kassiber"), "keep"),
        &Value::Int(1.into())
    );
    assert_eq!(
        field(field(&data, "kassiber"), "imported_value"),
        &s("opaque")
    );
    let (_, bytes) = export(&tmp, &book, "stored", None);
    let records = bytes
        .lines()
        .map(|line| kassiber_pytext::loads(line).unwrap())
        .collect::<Vec<_>>();
    let tx = records
        .iter()
        .find(|r| field(r, "type") == &s("tx"))
        .unwrap();
    assert_eq!(field(tx, "label"), &s("last"));
    assert_eq!(field(tx, "origin"), &s("old"));
    let output = records
        .iter()
        .find(|r| field(r, "type") == &s("output"))
        .unwrap();
    assert_eq!(field(output, "label"), &s("locked"));
    assert_eq!(field(output, "spendable"), &Value::Bool(false));
    assert_eq!(
        scalar(
            &book,
            "SELECT COUNT(*) FROM transaction_edit_events WHERE source='gui'"
        ),
        1
    );
    assert_eq!(
        scalar(&book, "SELECT journal_input_version FROM profiles"),
        2
    );
}
#[test]
fn matching_requires_one_transaction_and_wallet_exports_exclude_ambiguity() {
    let (tmp, book) = book();
    let file=write(&tmp,"{\"type\":\"tx\",\"ref\":\"SAME\",\"label\":\"shared\"}\n{\"type\":\"tx\",\"ref\":\"single\",\"label\":\"unique\"}\n");
    book.connection().execute_batch("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p2','w','Other','EUR','at','2025');INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('c','w','p2','Cross profile','address','2025');INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,kind,created_at) VALUES('cross','w','p2','c','cross','single','BTC','inbound',1000,0,'2025','deposit','2025');").unwrap();
    let preview = run(&book, Bip329::Preview { file: file.clone() });
    let rows = list(field(&preview, "rows"));
    assert_eq!(field(&rows[0], "match_status"), &s("ambiguous"));
    assert_eq!(list(field(&rows[0], "tag_effects")).len(), 2);
    assert_eq!(
        field(&list(field(&rows[0], "tag_effects"))[0], "action"),
        &s("skipped_ambiguous")
    );
    assert_eq!(field(&rows[1], "match_status"), &s("exact"));
    import(&book, &file, false);
    assert_eq!(scalar(&book, "SELECT COUNT(*) FROM transaction_tags"), 1);
    let (_, bytes) = export(&tmp, &book, "stored", Some(vec!["Alpha"]));
    assert!(bytes.is_empty());
    let (result, _) = export(&tmp, &book, "stored", Some(vec!["Beta"]));
    assert_eq!(field(&result, "exported"), &Value::Int(1.into()));
    import(&book, &file, true);
    assert_eq!(scalar(&book, "SELECT COUNT(*) FROM transaction_tags"), 3);
    book.connection()
        .execute_batch("UPDATE transactions SET external_id='same' WHERE id='b1';")
        .unwrap();
    let preview = run(&book, Bip329::Preview { file });
    let rows = list(field(&preview, "rows"));
    assert_eq!(
        field(&rows[0], "wallets"),
        &Value::List(vec![s("Alpha"), s("Beta")])
    );
    assert_eq!(
        list(field(&rows[0], "tag_effects"))
            .iter()
            .map(|e| field(e, "transaction_id").clone())
            .collect::<Vec<_>>(),
        vec![s("a1"), s("a2"), s("b1")]
    );
}
#[test]
fn export_keeps_stored_order_then_synthesized_and_first_object_wins() {
    let (tmp, book) = book();
    let file=write(&tmp,"{\"type\":\"tx\",\"ref\":\"single\",\"label\":\"Imported €\\n\\\"quote\\\"\",\"z\":1e-7,\"a\":{\"u\":\"\\ud800\"}}\n{\"type\":\"addr\",\"ref\":\"z\",\"label\":\"Address\"}\n");
    import(&book, &file, false);
    book.connection().execute_batch("UPDATE bip329_labels SET created_at='2025';UPDATE transactions SET note='Reviewed',review_status='completed',fiat_currency='EUR',fiat_rate_exact='123.00000001',fiat_value=1.0;").unwrap();
    let (result, bytes) = export(&tmp, &book, "all", None);
    assert_eq!(field(&result, "exported_stored"), &Value::Int(2.into()));
    assert_eq!(
        field(&result, "exported_synthesized"),
        &Value::Int(1.into())
    );
    let rows = bytes
        .lines()
        .map(|line| kassiber_pytext::loads(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(field(&rows[0], "type"), &s("addr"));
    assert_eq!(field(&rows[1], "ref"), &s("single"));
    assert_eq!(field(&rows[2], "ref"), &s("same"));
    assert_eq!(field(&rows[2], "value"), &Value::Int(123.into()));
    assert_eq!(
        field(field(&rows[2], "kassiber"), "transaction_id"),
        &s("a1")
    );
    assert!(bytes.contains("\\u20ac\\n\\\"quote\\\""));
    assert!(bytes.contains("\\ud800"));
    assert!(bytes.contains("1e-07"));
    assert!(bytes.ends_with('\n'));
    let (result, bytes) = export(&tmp, &book, "all", Some(vec!["Beta", "Alpha", "Beta"]));
    assert_eq!(field(&result, "exported"), &Value::Int(2.into()));
    assert_eq!(field(&result, "wallet"), &s("Beta, Alpha, Beta"));
    let first = kassiber_pytext::loads(bytes.lines().next().unwrap()).unwrap();
    assert_eq!(field(&first, "ref"), &s("single"));
}
#[test]
fn label_pages_keep_descending_id_ties_and_bind_scope() {
    let (tmp, book) = book();
    let file = write(
        &tmp,
        "{\"type\":\"tx\",\"ref\":\"single\"}\n{\"type\":\"tx\",\"ref\":\"same\"}\n",
    );
    import(&book, &file, false);
    book.connection().execute_batch("UPDATE bip329_labels SET created_at='2025',id=CASE ref WHEN 'single' THEN 'a' ELSE 'z' END;").unwrap();
    let first = run(
        &book,
        Bip329::List {
            cursor: None,
            limit: Some(1),
        },
    );
    assert_eq!(field(&list(field(&first, "records"))[0], "ref"), &s("same"));
    let Value::Str(cursor) = field(&first, "next_cursor") else {
        panic!("cursor")
    };
    let cursor = cursor.as_str().unwrap();
    let second = run(
        &book,
        Bip329::List {
            cursor: Some(cursor.into()),
            limit: Some(1),
        },
    );
    assert_eq!(
        field(&list(field(&second, "records"))[0], "ref"),
        &s("single")
    );
    assert_eq!(field(&second, "has_more"), &Value::Bool(false));
    let error = execute(
        book.connection(),
        &Scope {
            workspace_id: "other",
            profile_id: "p",
        },
        &Command::Bip329(Bip329::List {
            cursor: Some(cursor.into()),
            limit: Some(1),
        }),
    )
    .unwrap_err();
    assert_eq!(error.code, "validation");
}
#[test]
fn local_address_inventory_and_receive_outpoint_matches_preserve_sources() {
    let (tmp, book) = book();
    book.connection().execute_batch("UPDATE wallets SET config_json='{\"addresses\":[\"1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa\"],\"ownership_scan_to_index\":3.0}' WHERE id='a';").unwrap();
    let txid = "a".repeat(64);
    let raw =
        format!(r#"[{{"txid":"{txid}","vout":3,"address":"1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"}}]"#);
    book.connection()
        .execute(
            "UPDATE transactions SET external_id=?,raw_json=? WHERE id='a1'",
            params![txid, raw],
        )
        .unwrap();
    book.connection().execute("INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,chain,network,txid,vout,address,script_pubkey,asset,amount,outpoint,confirmation_status,first_seen_at,last_seen_at) VALUES('u','w','p','b','bitcoin','main',?,0,'bc1qsynthetic',NULL,'BTC',1000,'inventory:0','confirmed','2025','2025')",[&txid]).unwrap();
    let file=write(&tmp,&format!("{{\"type\":\"addr\",\"ref\":\"1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa\"}}\n{{\"type\":\"output\",\"ref\":\"{txid}:3\"}}\n{{\"type\":\"output\",\"ref\":\"{txid}:0\"}}\n"));
    let preview = run(&book, Bip329::Preview { file });
    let rows = list(field(&preview, "rows"));
    assert!(list(field(&preview, "warnings")).is_empty());
    for row in rows {
        assert_eq!(field(row, "match_status"), &s("exact"));
    }
    assert_eq!(field(&rows[0], "match_source"), &s("address"));
    assert_eq!(field(&rows[1], "match_source"), &s("address_list"));
    assert_eq!(field(&rows[2], "match_source"), &s("inventory"));
    book.connection().execute_batch("UPDATE wallets SET config_json='{\"addresses\":[\"1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa\"]}' WHERE id='b';").unwrap();
    let file=write(&tmp,&format!("{{\"type\":\"output\",\"ref\":\"{txid}:3\"}}\n{{\"type\":\"addr\",\"ref\":\"1a1zp1ep5qgefi2dmptftl5slmv7divfna\"}}\n"));
    let preview = run(&book, Bip329::Preview { file });
    let rows = list(field(&preview, "rows"));
    assert_eq!(field(&rows[0], "match_status"), &s("unmatched"));
    assert_eq!(field(&rows[1], "match_status"), &s("unmatched"));
}
#[test]
fn malformed_lines_and_failed_invalidation_leave_labels_tags_and_history_unchanged() {
    let (tmp, book) = book();
    let file = write(
        &tmp,
        "\r\n{\"type\":\"tx\",\"ref\":\"single\",\"label\":\"tag\"}\r\n{broken\r\n",
    );
    let err = execute(
        book.connection(),
        &scope(),
        &Command::Bip329(Bip329::Import {
            file: file.clone(),
            apply_ambiguous: false,
            source: "cli".into(),
        }),
    )
    .unwrap_err();
    assert_eq!(err.message, "Invalid BIP329 JSON on line 3");
    assert_eq!(scalar(&book, "SELECT COUNT(*) FROM bip329_labels"), 0);
    write(
        &tmp,
        "{\"type\":\"tx\",\"ref\":\"single\",\"label\":\"tag\"}\n",
    );
    book.connection().execute_batch("CREATE TRIGGER reject_input_version BEFORE UPDATE OF journal_input_version ON profiles BEGIN SELECT RAISE(FAIL,'blocked'); END;").unwrap();
    assert!(execute(
        book.connection(),
        &scope(),
        &Command::Bip329(Bip329::Import {
            file,
            apply_ambiguous: false,
            source: "cli".into()
        })
    )
    .is_err());
    for table in [
        "bip329_labels",
        "tags",
        "transaction_tags",
        "transaction_edit_events",
        "transaction_edit_fields",
    ] {
        assert_eq!(scalar(&book, &format!("SELECT COUNT(*) FROM {table}")), 0);
    }
}
#[test]
fn unported_descriptor_and_complex_graph_matching_refuse_before_writes() {
    let (tmp, book) = book();
    let file = write(
        &tmp,
        "{\"type\":\"tx\",\"ref\":\"single\",\"label\":\"tag\"}\n",
    );
    book.connection()
        .execute_batch(
            "UPDATE wallets SET config_json='{\"descriptor\":\"wpkh(unported)\"}' WHERE id='a';",
        )
        .unwrap();
    let error = execute(
        book.connection(),
        &scope(),
        &Command::Bip329(Bip329::Import {
            file: file.clone(),
            apply_ambiguous: false,
            source: "cli".into(),
        }),
    )
    .unwrap_err();
    assert_eq!(error.code, "not_implemented");
    assert_eq!(scalar(&book, "SELECT COUNT(*) FROM bip329_labels"), 0);
    book.connection().execute_batch("UPDATE wallets SET config_json='{}';UPDATE transactions SET raw_json='{\"vin\":[],\"vout\":[]}' WHERE id='b1';").unwrap();
    write(&tmp, "{\"type\":\"input\",\"ref\":\"single:0\"}\n");
    let error = execute(
        book.connection(),
        &scope(),
        &Command::Bip329(Bip329::Preview { file }),
    )
    .unwrap_err();
    assert_eq!(error.code, "not_implemented");
}

#[test]
fn conflicts_preserve_unknown_surrogate_keys_and_python_numeric_equality() {
    let (tmp, book) = book();
    let file = write(
        &tmp,
        r#"{"type":"tx","ref":"single","num":1,"big":9007199254740993,"\ud800":1,"object":{"keep":true}}"#,
    );
    import(&book, &file, false);
    write(
        &tmp,
        r#"{"type":"tx","ref":"single","num":1.0,"big":9007199254740992.0,"\ud800":2,"object":{"keep":false}}"#,
    );
    let preview = run(&book, Bip329::Preview { file });
    let rows = list(field(&preview, "rows"));
    let expected = kassiber_pytext::loads(r#"["data.big","data.\ud800","data.object"]"#).unwrap();
    assert_eq!(field(&rows[0], "conflicts"), &expected);
}
