use super::*;
use kassiber_envelope::get;

const MASTER: &str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
const XPRV: &str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
// wpkh(MASTER/0/*) receive 0..2 and change 0..1, and tr(MASTER/0/*) receive 1.
const RECEIVE: [&str; 3] = [
    "bc1qp5wfcq48h6d63wyy9qz0awtpfqwwv4sma86mhz",
    "bc1qrfxr69jqnhwufxgkqgcdep9prq4j4vuw2wyg0v",
    "bc1qhvd6suvqzjcu9pxjhrwhtrlj85ny3n2mqql5w4",
];
const RECEIVE_SCRIPT: [&str; 2] = [
    "00140d1c9c02a7be9ba8b8842804feb961481ce6561b",
    "00141a4c3d16409dddc499160230dc84a1182b2ab38e",
];
const CHANGE_SCRIPT: &str = "0014f09cb16010dc6d58dfafee3d3f9f027dc03be2c4";
const TAPROOT_RECEIVE_1: &str = "bc1pdqxk5pjfm2c5ellt67u9rjpajde2qlpjsgc244u6p4qpatf2ug6sv3hmk7";
const LISTED: &str = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
const BOLT11: &str = "lnbc1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdpl2pkx2ctnv5sxxmmwwd5kgetjypeh2ursdae8g6twvus8g6rfwvs8qun0dfjkxaq9qrsgq357wnc5r2ueh7ck6q93dj32dlqnls087fxdwk8qakdyafkq3yap9us6v52vjjsrvywa6rt52cm9r9zqt8r2t7mlcwspyetp5h2tztugp9lfyql";
const PAYMENT_HASH: &str = "0001020304050607080900010203040506070809000102030405060708090102";
const SP_DESCRIPTOR: &str = "sp(spscan1qpa55up5q9zn30790dw2pr7dpx0wn2ef9su2vcgn9jje5mwgvrukqyhxfs4kklqm4x58pywtcm2kzqrpxpj6mtt5rzpk2hyzgfhxclnekj2vrpm)";
const SP_BASE: &str = "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv";
const SP_CHANGE: &str = "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqma6khaxl7yel6xkzewummrwyze6k4grua2g6fvwtvja5eaf6apwgcl3l6s";
const SP_BEYOND: &str = "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgquktp7qshu3l828alg5l4v6cdt8nct63u8pef692q3avr8xneju0clzttpc";
const SP_OTHER: &str = "sp1qqw6vczcfpdh5nf5y2ky99kmqae0tr30hgdfg88parz50cp80wd2wqqlv6saelkk5snl4wfutyxrchpzzwm8rjp3z6q7apna59z9huq4x754e5atr";

fn book(wallets: &[(&str, &str, &str, String)]) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    let creation: Json =
        serde_json::from_str(include_str!("../../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch(
        "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');
         INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025');",
    )
    .unwrap();
    for (id, label, kind, config) in wallets {
        conn.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'w','p',?,?,?,'2025')",
            [id, label, kind, config.as_str()],
        )
        .unwrap();
    }
    conn
}

fn descriptor(kind: &str) -> String {
    serde_json::json!({ "descriptor": format!("{kind}({MASTER}/0/*)") }).to_string()
}

fn run(conn: &Connection, inputs: Inputs) -> Value {
    identify(conn, "p", &inputs).unwrap()
}

fn candidates(values: &[&str]) -> Inputs {
    Inputs {
        candidates: values.iter().map(|v| (*v).to_owned()).collect(),
        scan_to_index: 2,
        ..Inputs::default()
    }
}

fn results(report: &Value) -> Vec<Value> {
    match get(report, "results") {
        Some(Value::List(items)) => items.clone(),
        _ => panic!("results"),
    }
}

fn text(value: &Value, key: &str) -> String {
    py_str(get(value, key).unwrap_or(&Value::Null))
}

fn tx(
    conn: &Connection,
    id: &str,
    wallet: &str,
    external: &str,
    raw: &str,
    hash: Option<&str>,
    direction: &str,
) {
    conn.execute(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,amount_includes_fee,excluded,raw_json,payment_hash,created_at)
         VALUES(?1,'w','p',?2,?3,?1,'2025',?4,'BTC',1,0,0,0,?5,?6,'2025')",
        kassiber_store::rusqlite::params![id, wallet, external, direction, raw, hash],
    )
    .unwrap();
}

#[test]
fn identify_orders_derived_before_listed_and_flags_ambiguity() {
    let hot = serde_json::json!({ "descriptor": format!("wpkh({MASTER}/0/*)"), "addresses": [RECEIVE[1]] }).to_string();
    let conn = book(&[
        ("hot", "Hot", "descriptor", hot),
        (
            "list",
            "Watch list",
            "address",
            serde_json::json!({ "addresses": format!(" {LISTED} ,{}", RECEIVE[1]) }).to_string(),
        ),
    ]);
    let report = run(
        &conn,
        candidates(&[
            RECEIVE[1],
            LISTED,
            &LISTED.to_uppercase(),
            RECEIVE[0],
            "bc1qexternal0000000",
        ]),
    );
    let rows = results(&report);
    assert_eq!(
        rows.len(),
        4,
        "case variants of one bech32 address collapse"
    );
    assert_eq!(text(&rows[0], "canonical_wallet"), "Hot");
    assert_eq!(
        get(&rows[0], "ownership_ambiguous"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        text(&rows[0], "note"),
        "Owned by 'Hot' (receive #1). Also matches: Hot, Watch list."
    );
    let Some(Value::List(matches)) = get(&rows[0], "matches") else {
        panic!()
    };
    let order: Vec<(String, String)> = matches
        .iter()
        .map(|m| (text(m, "wallet"), text(m, "match_source")))
        .collect();
    assert_eq!(
        order,
        [
            ("Hot", "derived"),
            ("Hot", "address_list"),
            ("Watch list", "address_list")
        ]
        .map(|(w, m)| (w.to_owned(), m.to_owned()))
    );
    assert_eq!(text(&rows[1], "note"), "Owned by 'Watch list' (address).");
    assert_eq!(text(&rows[2], "note"), "Owned by 'Hot' (receive #0).");
    assert_eq!(text(&rows[3], "status"), "external");
    let summary = get(&report, "summary").unwrap();
    assert_eq!(get(summary, "owned"), Some(&Value::from(3)));
    assert_eq!(get(summary, "wallets_scanned"), Some(&Value::from(2)));
}

#[test]
fn identify_scans_retired_policies_and_reports_bad_depths() {
    let conn = book(&[(
        "cold",
        "Cold",
        "descriptor",
        serde_json::json!({ "descriptor": format!("wpkh({MASTER}/0/*)"), "ownership_scan_to_index": "deep" }).to_string(),
    )]);
    conn.execute_batch(&format!(
        "INSERT INTO wallet_policy_epochs(id,workspace_id,profile_id,wallet_id,chain,network,status,private_material_json,created_at)
         VALUES('e1','w','p','cold','bitcoin','main','retired','{}','2024'),
               ('e2','w','p','cold','bitcoin','main','retired','{}','2024'),
               ('e3','w','p','cold','bitcoin','main','active','{}','2024');",
        serde_json::json!({ "descriptor": format!("tr({MASTER}/0/*)") }),
        serde_json::json!({ "addresses": [LISTED], "descriptor": "x", "scan_to_index": "?" }),
        serde_json::json!({ "addresses": ["1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2"] }),
    ))
    .unwrap();
    let report = run(
        &conn,
        candidates(&[
            TAPROOT_RECEIVE_1,
            LISTED,
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
        ]),
    );
    let rows = results(&report);
    let Some(Value::List(matches)) = get(&rows[0], "matches") else {
        panic!()
    };
    assert_eq!(text(&matches[0], "match_source"), "derived_history");
    let Some(Value::List(matches)) = get(&rows[1], "matches") else {
        panic!()
    };
    assert_eq!(text(&matches[0], "match_source"), "address_history");
    assert_eq!(
        text(&rows[2], "status"),
        "external",
        "only retired epochs are historic"
    );
    assert_eq!(
        get(&report, "warnings"),
        Some(&Value::List(vec![
            s("Wallet 'Cold': ownership scan depth is invalid"),
            s("Wallet 'Cold': historic scan floor is invalid"),
        ]))
    );
}

#[test]
fn identify_derivation_ceiling_follows_depth_and_inventory() {
    let gapless =
        serde_json::json!({ "descriptor": format!("wpkh({MASTER}/0/*)"), "gap_limit": 1 })
            .to_string();
    let conn = book(&[("hot", "Hot", "descriptor", gapless)]);
    let depth = |scan_to_index| {
        let report = run(
            &conn,
            Inputs {
                scan_to_index,
                ..candidates(&[RECEIVE[2]])
            },
        );
        text(&results(&report)[0], "status")
    };
    assert_eq!(depth(1), "external");
    assert_eq!(depth(2), "owned");
    // A used index from the output inventory extends the ceiling by the gap.
    conn.execute_batch(&format!(
        "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,confirmation_status,address,branch_label,address_index,anon_history_json,first_seen_at,last_seen_at,raw_json)
         VALUES('u','w','p','hot','bitcoin','main','BTC',1,'{t}',0,'{t}:0','confirmed','{a}','receive',1,'[]','2025','2025','{{}}');",
        t = "a".repeat(64),
        a = RECEIVE[1],
    ))
    .unwrap();
    assert_eq!(depth(0), "owned");
    assert_eq!(
        Inputs::from_args(Some(&dict([("scan_to_index", Value::from(99_999_999))])))
            .unwrap()
            .scan_to_index,
        MAX_SCAN_TO_INDEX
    );
    assert_eq!(
        Inputs::from_args(Some(&dict([("scan_to_index", Value::from(-5))])))
            .unwrap()
            .scan_to_index,
        -1
    );
    let report = run(
        &conn,
        Inputs {
            scan_to_index: -1,
            ..candidates(&[])
        },
    );
    assert_eq!(
        get(get(&report, "summary").unwrap(), "scan_to_index"),
        Some(&Value::from(0))
    );
}

#[test]
fn identify_classifies_transaction_legs() {
    let conn = book(&[("hot", "Hot", "descriptor", descriptor("wpkh"))]);
    let graph = |txid: &str, inputs: &[(&str, Option<&str>)], outputs: &[&str]| {
        serde_json::json!({
            "txid": txid,
            "vin": inputs.iter().map(|(prev, script)| serde_json::json!({"txid": prev, "vout": 0, "prevout": script.map(|s| serde_json::json!({"scriptpubkey": s}))})).collect::<Vec<_>>(),
            "vout": outputs.iter().map(|s| serde_json::json!({"scriptpubkey": s, "value": 1})).collect::<Vec<_>>(),
        })
        .to_string()
    };
    let external = format!("0014{}", "ee".repeat(20));
    let [pay, receipt, consolidate, none] = ["1", "2", "3", "4"].map(|c| c.repeat(64));
    tx(
        &conn,
        "a",
        "hot",
        &pay,
        &graph(
            &pay,
            &[("9", Some(RECEIVE_SCRIPT[0]))],
            &[&external, CHANGE_SCRIPT, "6a00", ""],
        ),
        None,
        "outbound",
    );
    tx(
        &conn,
        "b",
        "hot",
        &receipt,
        &graph(&receipt, &[("9", Some(&external))], &[RECEIVE_SCRIPT[1]]),
        None,
        "inbound",
    );
    tx(
        &conn,
        "c",
        "hot",
        &consolidate,
        &graph(
            &consolidate,
            &[("9", Some(RECEIVE_SCRIPT[0]))],
            &[RECEIVE_SCRIPT[1]],
        ),
        None,
        "outbound",
    );
    tx(
        &conn,
        "d",
        "hot",
        &none,
        &graph(&none, &[("9", Some(&external))], &[&external]),
        None,
        "outbound",
    );
    let report = run(
        &conn,
        Inputs {
            txids: vec![pay, receipt, consolidate, none, "5".repeat(64)],
            ..candidates(&[])
        },
    );
    let rows = results(&report);
    let classes: Vec<String> = rows.iter().map(|row| text(row, "classification")).collect();
    assert_eq!(
        classes,
        [
            "outbound_payment",
            "inbound_receipt",
            "self_transfer",
            "external",
            "unknown"
        ]
    );
    assert_eq!(
        get(&rows[0], "external_outputs"),
        Some(&Value::from(1)),
        "OP_RETURN and fee outputs are not recipients"
    );
    assert_eq!(get(&rows[0], "owned_outputs"), Some(&Value::from(1)));
    assert_eq!(text(&rows[0], "match_source"), "local_tx");
}

#[test]
fn identify_unverified_legs_and_inventory_txids() {
    let index = Index::default();
    let legs = Legs {
        inputs: vec![(s("x:0"), Value::Null)],
        outputs: vec![(Value::from(0), s(RECEIVE_SCRIPT[0]))],
        chain: "bitcoin".into(),
        network: String::new(),
        source: "chain".into(),
    };
    let row = classify_txid(&"6".repeat(64), "", "", &index, Some(&legs));
    assert_eq!(text(&row, "classification"), "external");
    let mut index = Index::default();
    let item = Match {
        wallet_id: "hot".into(),
        wallet_label: "Hot".into(),
        account: String::new(),
        chain: "bitcoin".into(),
        network: "main".into(),
        branch_label: "receive".into(),
        address_index: Value::from(0),
        derivation_path: Value::Null,
        source: "derived",
        wallet_kind: "descriptor".into(),
    };
    index.add_script(Some(RECEIVE_SCRIPT[0].into()), &item);
    let row = classify_txid(&"6".repeat(64), "", "", &index, Some(&legs));
    assert_eq!(
        text(&row, "classification"),
        "undetermined",
        "an on-chain input without script is unresolved"
    );
    let local = Legs {
        source: "local_tx".into(),
        ..legs
    };
    assert_eq!(
        text(
            &classify_txid(&"6".repeat(64), "", "", &index, Some(&local)),
            "classification"
        ),
        "inbound_receipt"
    );
    index.note_txid(&"7".repeat(64), &item);
    let row = classify_txid(&"7".repeat(64), "", "", &index, None);
    assert_eq!(
        text(&row, "note"),
        "Recorded against 'Hot'; per-leg breakdown needs on-chain verification."
    );
}

#[test]
fn identify_core_receive_outpoints_need_an_exclusive_owner() {
    let listed = serde_json::json!({ "addresses": [LISTED] }).to_string();
    let conn = book(&[
        ("list", "List", "address", listed.clone()),
        (
            "other",
            "Other",
            "address",
            serde_json::json!({ "addresses": [RECEIVE[0]] }).to_string(),
        ),
    ]);
    let core = "4".repeat(64);
    let observations = serde_json::json!([
        {"txid": core.to_uppercase(), "address": LISTED, "vout": 2},
        {"txid": core, "address": RECEIVE[0], "vout": true},
    ])
    .to_string();
    tx(&conn, "core", "list", &core, &observations, None, "inbound");
    let spend = "8".repeat(64);
    let graph = serde_json::json!({"txid": spend, "vin": [{"txid": core, "vout": 2}], "vout": [{"scriptpubkey": format!("0014{}", "ee".repeat(20)), "value": 1}]}).to_string();
    tx(&conn, "spend", "list", &spend, &graph, None, "outbound");
    let rows = results(&run(
        &conn,
        Inputs {
            txids: vec![spend.clone()],
            ..candidates(&[])
        },
    ));
    assert_eq!(get(&rows[0], "owned_inputs"), Some(&Value::from(1)));
    // A second owner of the receiving address withdraws the exact outpoint.
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id='other'",
        [&listed],
    )
    .unwrap();
    let rows = results(&run(
        &conn,
        Inputs {
            txids: vec![spend],
            ..candidates(&[])
        },
    ));
    assert_eq!(get(&rows[0], "owned_inputs"), Some(&Value::from(0)));
}

#[test]
fn identify_tokens_keep_private_and_invoice_material_out() {
    let conn = book(&[]);
    let lightning = format!("lightning:{}", BOLT11.to_uppercase());
    let broken = format!("{}q", &BOLT11[..BOLT11.len() - 1]);
    let report = run(
        &conn,
        Inputs {
            text: Some(format!("{XPRV}\n# note\r\nlno1{q}\nsatoshi@example.com\n{h}:1\n{MASTER}\nnot an address\n{BOLT11}", q = "q".repeat(60), h = "a".repeat(64))),
            candidates: vec![lightning, broken, format!("wpkh({XPRV}/0/*)")],
            txids: vec!["g".repeat(64), "ab".repeat(32)],
            ..candidates(&[])
        },
    );
    let rendered = kassiber_pyjson::dumps(&report, &Default::default()).unwrap();
    assert!(!rendered.contains(XPRV) && !rendered.to_lowercase().contains(&BOLT11[20..]));
    let kinds: Vec<(String, String)> = results(&report)
        .iter()
        .map(|r| (text(r, "type"), text(r, "status")))
        .collect();
    let expected = [
        ("txid", "unknown"),
        ("lightning_invoice", "unknown"),
        ("txid", "invalid"),
        ("lightning_invoice", "invalid"),
        ("private_key", "unsupported"),
        ("private_key", "unsupported"),
        ("lightning_offer", "unsupported"),
        ("lightning_address", "unsupported"),
        ("outpoint", "unsupported"),
        ("extended_public_key", "unsupported"),
        ("address", "invalid"),
    ];
    assert_eq!(kinds, expected.map(|(a, b)| (a.to_owned(), b.to_owned())));
}

#[test]
fn identify_lightning_history_names_direction() {
    let conn = book(&[
        ("n1", "Node", "lnd", "{}".into()),
        ("n2", "Second", "coreln", "{}".into()),
    ]);
    let classify = || {
        text(
            &results(&run(&conn, candidates(&[BOLT11])))[0],
            "classification",
        )
    };
    assert_eq!(classify(), "unknown");
    tx(
        &conn,
        "a",
        "n1",
        "ln-a",
        "{}",
        Some(PAYMENT_HASH),
        "outbound",
    );
    assert_eq!(classify(), "outbound_payment");
    tx(
        &conn,
        "b",
        "n2",
        "ln-b",
        "{}",
        Some(PAYMENT_HASH),
        "inbound",
    );
    assert_eq!(classify(), "self_transfer");
    conn.execute("UPDATE transactions SET direction='inbound'", [])
        .unwrap();
    assert_eq!(classify(), "inbound_receipt");
}

#[test]
fn identify_silent_payment_addresses_need_scan_and_spend_keys() {
    let conn = book(&[(
        "sp",
        "Silent",
        "silent-payment",
        serde_json::json!({ "sp_descriptor": SP_DESCRIPTOR, "network": "main" }).to_string(),
    )]);
    let rows = results(&run(
        &conn,
        candidates(&[SP_BASE, SP_CHANGE, SP_BEYOND, SP_OTHER]),
    ));
    let statuses: Vec<String> = rows.iter().map(|r| text(r, "status")).collect();
    assert_eq!(statuses, ["owned", "owned", "unknown", "external"]);
    assert_eq!(text(&rows[1], "note"), "Owned by 'Silent' (change label).");
    assert_eq!(
        text(&rows[2], "note"),
        "Uses the scan key of 'Silent', but not its spend key or a label it checked (0-50)."
    );
    let two_key = "sp(KwjfjB2gU7NRkcRdgUSfaa83QWAFnfFxvxZK1AfkwSfHCYZnmrM2,025cc9856d6f8375350e123978daac200c260cb5b5ae83106cab90484dcd8fcf36)";
    let conn = book(&[(
        "sp",
        "Silent",
        "silent-payment",
        serde_json::json!({ "sp_descriptor": two_key }).to_string(),
    )]);
    assert_eq!(
        text(&results(&run(&conn, candidates(&[SP_BASE])))[0], "status"),
        "owned"
    );
    let conn = book(&[(
        "sp",
        "Silent",
        "silent-payment",
        serde_json::json!({ "sp_descriptor": "sp(spscan1qqqq)" }).to_string(),
    )]);
    assert_eq!(
        text(&results(&run(&conn, candidates(&[SP_BASE])))[0], "status"),
        "unknown"
    );
    let conn = book(&[(
        "sp",
        "Silent",
        "silent-payment",
        serde_json::json!({ "sp_descriptor": SP_DESCRIPTOR, "network": "testnet" }).to_string(),
    )]);
    assert_eq!(
        text(&results(&run(&conn, candidates(&[SP_BASE])))[0], "status"),
        "external"
    );
}

#[test]
fn csv_harvest_prefers_named_columns_and_recovers_from_reader_errors() {
    let txid = "c".repeat(64);
    let harvested = harvest_csv(&format!(
        "\u{feff}Note;TXID;Address\nx 1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2;{txid};{LISTED}\n"
    ));
    assert_eq!(
        harvested,
        [
            LISTED.to_owned(),
            txid.clone(),
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2".to_owned()
        ]
    );
    let harvested = harvest_csv(&format!(
        "a\rb,{LISTED}\nword,1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN3\n"
    ));
    assert_eq!(harvested, [LISTED]);
    let many: String = (0..MAX_HARVEST_CANDIDATES + 3)
        .map(|n| format!("{n:064x}\n"))
        .collect();
    assert_eq!(harvest_csv(&many).len(), MAX_HARVEST_CANDIDATES + 1);
}

#[test]
fn csv_raw_split_collapses_delimiter_runs_like_re_split() {
    assert_eq!(raw_split("a,,b"), ["a", "b"]);
    assert_eq!(raw_split(",a; b\t"), ["", "a", "b", ""]);
    assert_eq!(raw_split(""), [""]);
}
