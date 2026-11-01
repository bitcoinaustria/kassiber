use kassiber_envelope::{get, s};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::params, Book};
use kassiber_transactions::{
    listing::{list, Options},
    Scope,
};

fn book() -> (tempfile::TempDir, Book) {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    book.connection().execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025'),('other','w','Other','EUR','at','2025');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wal','w','p','Cold','address','{}','2025'),('exchange','w','p','Exchange','kraken','{}','2025'),('ln','w','p','Lightning','lnd','{}','2025'),('liquid','w','p','Liquid','liquid','{}','2025'),('foreign','w','other','Cold','address','{}','2025');").unwrap();
    for (id, amount, fee, date, created) in [
        ("a", 10, 1, "2025-02-01T00:00:00Z", "2025-02-02"),
        ("b", 10, 1, "2025-02-01T00:00:00Z", "2025-02-02"),
        ("c", 10, 1, "2025-02-01T00:00:00Z", "2025-02-01"),
        ("d", 20, 2, "2025-01-01T00:00:00Z", "2025-02-02"),
    ] {
        book.connection().execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,confirmed_at,kind,created_at,fiat_value) VALUES(?,'w','p','wal',?,?,'BTC','inbound',?,?,?,?,'deposit',?,?)",params![id,id,format!("external-{id}"),amount,fee,date,date,created,amount]).unwrap();
    }
    (tmp, book)
}
fn run(book: &Book, options: &Options) -> Value {
    list(
        book.connection(),
        &Scope {
            workspace_id: "w",
            profile_id: "p",
        },
        options,
    )
    .unwrap()
}
fn field(row: &Value, key: &str) -> Value {
    get(row, key).unwrap().clone()
}
fn ids(page: &Value) -> Vec<String> {
    let Value::List(rows) = get(page, "records").unwrap() else {
        panic!("records")
    };
    rows.iter()
        .map(|row| match get(row, "id").unwrap() {
            Value::Str(t) => t.as_str().unwrap().into(),
            _ => panic!("id"),
        })
        .collect()
}
fn filtered(book: &Book, key: &str, val: &str) -> Vec<String> {
    let mut options = Options::default();
    *match key {
        "flow" => &mut options.flow,
        "status" => &mut options.status,
        "quick" => &mut options.quick,
        "payment" => &mut options.payment_method,
        "network" => &mut options.network,
        _ => panic!("filter"),
    } = Some(val.into());
    ids(&run(book, &options))
}

#[test]
fn transaction_pages_keep_all_sort_ties_counts_and_bound_cursors() {
    let (_tmp, book) = book();
    for (sort, order, expected) in [
        ("occurred-at", "desc", vec!["b", "a", "c", "d"]),
        ("occurred-at", "asc", vec!["d", "c", "a", "b"]),
        ("amount", "desc", vec!["d", "b", "a", "c"]),
        ("amount", "asc", vec!["b", "a", "c", "d"]),
        ("fee", "desc", vec!["d", "b", "a", "c"]),
        ("fee", "asc", vec!["b", "a", "c", "d"]),
        ("fiat-value", "desc", vec!["d", "b", "a", "c"]),
        ("fiat-value", "asc", vec!["b", "a", "c", "d"]),
    ] {
        let mut options = Options {
            limit: Some(1),
            sort: Some(sort.into()),
            order: Some(order.into()),
            ..Default::default()
        };
        let mut actual = Vec::new();
        loop {
            let page = run(&book, &options);
            assert_eq!(field(&page, "count"), Value::Int(4.into()));
            assert_eq!(field(&page, "total"), Value::Int(4.into()));
            actual.extend(ids(&page));
            assert_eq!(
                field(&page, "has_more"),
                Value::Bool(actual.len() < expected.len()),
                "a full final page must not advertise an empty follow-up page"
            );
            let next = field(&page, "next_cursor");
            if next == Value::Null {
                assert_eq!(field(&page, "has_more"), Value::Bool(false));
                break;
            }
            assert_eq!(field(&page, "has_more"), Value::Bool(true));
            let Value::Str(next) = next else {
                panic!("cursor")
            };
            options.cursor = Some(next.as_str().unwrap().into());
            let mut changed = Options {
                cursor: options.cursor.clone(),
                sort: Some(sort.into()),
                order: Some(order.into()),
                with_fees: true,
                ..Default::default()
            };
            assert_eq!(
                list(
                    book.connection(),
                    &Scope {
                        workspace_id: "w",
                        profile_id: "p"
                    },
                    &changed
                )
                .unwrap_err()
                .code,
                "validation"
            );
            changed.with_fees = false;
            changed.order = Some(if order == "asc" { "desc" } else { "asc" }.into());
            assert!(list(
                book.connection(),
                &Scope {
                    workspace_id: "w",
                    profile_id: "p"
                },
                &changed
            )
            .is_err());
            changed.order = Some(order.into());
            changed.sort = Some(if sort == "amount" { "fee" } else { "amount" }.into());
            assert!(list(
                book.connection(),
                &Scope {
                    workspace_id: "w",
                    profile_id: "p"
                },
                &changed
            )
            .is_err());
            assert!(actual.len() <= 4, "cursor must advance");
        }
        assert_eq!(actual, expected, "{sort} {order}");
    }
}

#[test]
fn transaction_filters_keep_profile_wallet_dates_assets_and_normalized_ids() {
    let (_tmp, book) = book();
    let conn = book.connection();
    conn.execute_batch("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,created_at) VALUES('outside','w','other','foreign','outside','BTC','inbound',10,1,'2025-02-01T00:00:00Z','2025');
        UPDATE transactions SET wallet_id='exchange',asset='LBTC',direction='outbound',external_id='ABCDEF' WHERE id='a';").unwrap();
    assert_eq!(ids(&run(&book, &Options::default())), ["b", "a", "c", "d"]);
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                wallet: Some("exchange".into()),
                ..Default::default()
            }
        )),
        ["a"]
    );
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                asset: Some("lbtc".into()),
                direction: Some("outbound".into()),
                ..Default::default()
            }
        )),
        ["a"]
    );
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                asset: Some("BTC".into()),
                direction: Some("outbound".into()),
                ..Default::default()
            }
        )),
        Vec::<String>::new()
    );
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                txids: vec![" , ABCDEF,a,abcdef ".into(), "EXTERNAL-C, B".into()],
                ..Default::default()
            }
        )),
        ["b", "a", "c"]
    );
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                start: Some("2025-01-01".into()),
                end: Some("2025-01-31T23:00:00-01:00".into()),
                ..Default::default()
            }
        )),
        ["b", "a", "c", "d"]
    );
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                start: Some("2025-02-01T00:00:00.000001Z".into()),
                ..Default::default()
            }
        )),
        Vec::<String>::new()
    );
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                end: Some("2025-02-01T00:00:00.000001Z".into()),
                ..Default::default()
            }
        )),
        ["b", "a", "c", "d"]
    );
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                period: Some("all".into()),
                ..Default::default()
            }
        )),
        ["b", "a", "c", "d"]
    );
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                period: Some("ytd".into()),
                ..Default::default()
            }
        )),
        Vec::<String>::new()
    );
    conn.execute("UPDATE wallets SET label='cold' WHERE id='exchange'", [])
        .unwrap();
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                wallet: Some("wal".into()),
                ..Default::default()
            }
        )),
        ["b", "c", "d"]
    );
    let options = Options {
        wallet: Some("Cold".into()),
        ..Default::default()
    };
    assert_eq!(
        list(
            conn,
            &Scope {
                workspace_id: "w",
                profile_id: "p"
            },
            &options
        )
        .unwrap_err()
        .message,
        "Wallet label 'Cold' is ambiguous"
    );
}

#[test]
fn transaction_quick_status_and_price_filters_keep_quarantine_precedence() {
    let (_tmp, book) = book();
    book.connection().execute_batch("UPDATE transactions SET review_status='complete',confirmed_at=NULL,fee=0 WHERE id='a';
        UPDATE transactions SET review_status='FAILED',fiat_value=NULL,fiat_rate_exact='0.00001' WHERE id='b';
        UPDATE transactions SET review_status='completed',fiat_value=0,fiat_rate=0,fiat_value_exact='0' WHERE id='c';
        UPDATE transactions SET confirmed_at=NULL,external_id='0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' WHERE id='d';
        INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('c','w','p','blocked','2025');").unwrap();
    for (status, expected) in [
        ("completed", vec!["a"]),
        ("pending", vec!["d"]),
        ("review", vec!["c"]),
        ("failed", vec!["b"]),
    ] {
        assert_eq!(filtered(&book, "status", status), expected);
    }
    assert_eq!(filtered(&book, "quick", "review_queue"), ["b", "c", "d"]);
    assert_eq!(filtered(&book, "quick", "failed_import"), ["b"]);
    assert_eq!(filtered(&book, "quick", "missing_price"), ["c"]);
    assert_eq!(filtered(&book, "quick", "no_explorer_id"), ["b", "a", "c"]);
    assert_eq!(
        ids(&run(
            &book,
            &Options {
                with_fees: true,
                ..Default::default()
            }
        )),
        ["b", "c", "d"]
    );
    book.connection()
        .execute("UPDATE transactions SET kind='transfer' WHERE id='a'", [])
        .unwrap();
    assert_eq!(filtered(&book, "quick", "external_flow"), ["b", "c", "d"]);
}

#[test]
fn transaction_payment_and_network_filters_keep_family_precedence_and_json_guards() {
    let (_tmp, book) = book();
    book.connection().execute_batch("UPDATE transactions SET wallet_id='ln' WHERE id='b'; UPDATE transactions SET wallet_id='exchange' WHERE id='c'; UPDATE transactions SET wallet_id='liquid' WHERE id='d';
        UPDATE wallets SET config_json='{\"network\":\"RegTest\",\"chain\":\"bitcoin\"}' WHERE id='wal';").unwrap();
    for (method, expected) in [
        ("On-chain", vec!["a"]),
        ("Exchange", vec!["c"]),
        ("Lightning", vec!["b"]),
        ("Liquid", vec!["d"]),
    ] {
        assert_eq!(filtered(&book, "payment", method), expected);
        assert_eq!(filtered(&book, "network", method), expected);
    }
    assert_eq!(filtered(&book, "network", " regtest "), ["a"]);
    assert_eq!(filtered(&book, "network", "BITCOIN"), ["a"]);
    assert_eq!(filtered(&book, "network", "btc"), ["b", "a", "c", "d"]);
    book.connection().execute_batch("UPDATE transactions SET asset='LBTC' WHERE id='b'; UPDATE wallets SET config_json='malformed' WHERE id='wal';").unwrap();
    assert_eq!(filtered(&book, "payment", "Liquid"), ["b", "d"]);
    assert_eq!(filtered(&book, "network", "regtest"), Vec::<String>::new());
}

#[test]
fn transaction_flows_use_only_fresh_profile_scoped_custody_projections() {
    let (_tmp, book) = book();
    book.connection().execute_batch("UPDATE transactions SET direction='outbound' WHERE id='a'; UPDATE transactions SET kind='swap' WHERE id='c'; UPDATE transactions SET kind='peg-in' WHERE id='d';
        INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,created_at) VALUES(printf('%064d',1),'w','p','conversion','a','b','BTC','LBTC',1,1,'eligible','2025');").unwrap();
    assert_eq!(filtered(&book, "flow", "incoming"), ["b"]);
    assert_eq!(filtered(&book, "flow", "outgoing"), ["a"]);
    assert_eq!(filtered(&book, "flow", "swap"), ["c", "d"]);
    assert_eq!(filtered(&book, "flow", "layer-transition"), ["d"]);
    book.connection().execute_batch("UPDATE profiles SET last_processed_at='2025',journal_input_version=2,last_processed_input_version=1 WHERE id='p'").unwrap();
    assert_eq!(filtered(&book, "flow", "incoming"), ["b"]);
    book.connection()
        .execute(
            "UPDATE profiles SET last_processed_input_version=2,last_processed_tx_count=4 WHERE id='p'",
            [],
        )
        .unwrap();
    assert_eq!(filtered(&book, "flow", "incoming"), ["b"]);
    book.connection()
        .execute(
            "UPDATE profiles SET journal_semantics_version=? WHERE id='p'",
            [kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION],
        )
        .unwrap();
    assert_eq!(filtered(&book, "flow", "incoming"), Vec::<String>::new());
    assert_eq!(filtered(&book, "flow", "outgoing"), Vec::<String>::new());
    assert_eq!(filtered(&book, "flow", "swap"), ["b", "a", "c", "d"]);
    // A projection in another profile never grants a relationship here.
    book.connection()
        .execute(
            "UPDATE journal_custody_economic_relations SET profile_id='other'",
            [],
        )
        .unwrap();
    assert_eq!(filtered(&book, "flow", "incoming"), ["b"]);
    book.connection().execute_batch("INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,created_at) VALUES(printf('%064d',2),'w','p','a','b',printf('%064d',3),0,1,printf('%064d',4),0,1,'mainnet','mainnet','onchain','onchain','BTC','BTC','internal_reviewed','eligible','reviewed','2025')").unwrap();
    assert_eq!(filtered(&book, "flow", "transfer"), ["b", "a"]);
    assert_eq!(filtered(&book, "flow", "incoming"), Vec::<String>::new());
    book.connection()
        .execute(
            "UPDATE profiles SET last_processed_at=NULL WHERE id='p'",
            [],
        )
        .unwrap();
    assert_eq!(filtered(&book, "flow", "transfer"), Vec::<String>::new());
}

#[test]
fn transaction_rows_preserve_msat_decimal_rounding_tags_and_observed_fields() {
    let (_tmp, book) = book();
    book.connection().execute_batch("UPDATE transactions SET amount=9007199254740997,fee=1,fiat_rate=-1,fiat_value=0,fiat_rate_exact='123.000',fiat_value_exact='0.00',note='',description=NULL,excluded=1,kind_override='income' WHERE id='a';
        INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('z','w','p','z','Last','2025'),('a','w','p','a','First','2025'); INSERT INTO transaction_tags(transaction_id,tag_id) VALUES('a','z'),('a','a');").unwrap();
    let page = run(
        &book,
        &Options {
            txids: vec!["a".into()],
            ..Default::default()
        },
    );
    let Value::List(rows) = field(&page, "records") else {
        panic!("records")
    };
    let row = &rows[0];
    assert_eq!(
        field(row, "amount_msat"),
        Value::Int(9_007_199_254_740_997_i64.into())
    );
    assert_eq!(field(row, "amount"), Value::Float(90071.99254740997));
    assert_eq!(field(row, "fee_msat"), Value::Int(1.into()));
    assert_eq!(field(row, "fee"), Value::Float(1e-11));
    assert_eq!(field(row, "fiat_rate"), Value::Null);
    assert_eq!(field(row, "fiat_value"), Value::Null);
    assert_eq!(field(row, "fiat_rate_exact"), s("123.000"));
    assert_eq!(field(row, "description"), s(""));
    assert_eq!(field(row, "note"), s(""));
    assert_eq!(field(row, "kind"), s("deposit"));
    assert_eq!(field(row, "excluded"), Value::Bool(true));
    let Value::List(tags) = field(row, "tags") else {
        panic!("tags")
    };
    assert_eq!(
        tags.iter()
            .map(|row| field(row, "label"))
            .collect::<Vec<_>>(),
        [s("First"), s("Last")]
    );
    assert!(get(row, "_created_at").is_none());
    assert!(get(row, "raw_json").is_none());
    assert!(get(row, "wallet_config_json").is_none());
}

#[test]
fn transaction_validation_and_empty_pages_are_read_only() {
    let (_tmp, book) = book();
    let conn = book.connection();
    let before = conn.total_changes();
    for options in [
        Options {
            limit: Some(0),
            ..Default::default()
        },
        Options {
            limit: Some(1001),
            ..Default::default()
        },
        Options {
            direction: Some("bad".into()),
            ..Default::default()
        },
        Options {
            sort: Some("bad".into()),
            ..Default::default()
        },
        Options {
            order: Some("bad".into()),
            ..Default::default()
        },
        Options {
            start: Some("bad".into()),
            ..Default::default()
        },
        Options {
            period: Some("bad".into()),
            ..Default::default()
        },
        Options {
            wallet: Some("absent".into()),
            ..Default::default()
        },
        Options {
            payment_method: Some("bad".into()),
            ..Default::default()
        },
        Options {
            cursor: Some("broken".into()),
            ..Default::default()
        },
    ] {
        assert!(list(
            conn,
            &Scope {
                workspace_id: "w",
                profile_id: "p"
            },
            &options
        )
        .is_err());
    }
    let empty = run(
        &book,
        &Options {
            txids: vec!["absent".into()],
            ..Default::default()
        },
    );
    assert!(ids(&empty).is_empty());
    assert_eq!(field(&empty, "count"), Value::Int(0.into()));
    assert_eq!(field(&empty, "next_cursor"), Value::Null);
    assert_eq!(conn.total_changes(), before);
}

#[test]
fn narrow_pages_keep_rows_with_null_text_primary_keys() {
    let (_tmp, book) = book();
    // A SQLite TEXT PRIMARY KEY on a rowid table permits NULL. The internal
    // page lookup must still retain each physical row without joining NULLs.
    book.connection()
        .execute("UPDATE transactions SET id=NULL WHERE id IN ('a','b')", [])
        .unwrap();
    let page = run(&book, &Options::default());
    let Value::List(records) = get(&page, "records").unwrap() else {
        panic!("records")
    };
    assert_eq!(records.len(), 4);
    assert_eq!(field(&page, "total"), Value::Int(4.into()));
    assert_eq!(
        records
            .iter()
            .filter(|row| get(row, "id") == Some(&Value::Null))
            .count(),
        2
    );
}
