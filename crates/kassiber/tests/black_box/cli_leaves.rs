//! The final native CLI leaves (`journals list`, `journals events list|get`,
//! `journals transfers list`, `journals quarantined`, `journals quarantine
//! show|clear|resolve`, `wallets identify|analyze-file|reveal-descriptor` and
//! `maintenance purge-legacy-ledger`) on synthetic books, pinned to golden
//! transcripts. Replaces `tests/test_rust_cli_leaves.py`, whose Python
//! comparison these goldens froze.

use crate::support::{Sandbox, Transcript};

const CLOCK: &str = "2026-10-31T12:00:00Z";
const DESCRIPTOR: &str = "wpkh([d34db33f/84h/0h/0h]xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz/0/*)";
const LISTED: &str = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

/// `tests/test_journal_storage_format.py::_book`: five priced rows in one
/// manual wallet, FIFO, generic country, with the context selected.
const BOOK: &str = "
INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2020');
INSERT INTO profiles(id,workspace_id,label,gains_algorithm,tax_country,fiat_currency,created_at)
 VALUES('p','w','P','fifo','generic','EUR','2020');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at)
 VALUES('wallet','w','p','Wallet','manual','2020');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,
 amount,fee,kind,fiat_currency,fiat_rate,fiat_rate_exact,pricing_source_kind,pricing_quality,
 pricing_provider,raw_json,created_at) VALUES
 ('a','w','p','wallet','a','2024-01-01T00:00:00Z','inbound','BTC',100000000000,0,'buy','EUR',10000.0,'10000','manual','exact','reviewed-receipt','{}','2020'),
 ('b','w','p','wallet','b','2024-02-01T00:00:00Z','inbound','BTC',100000000000,0,'buy','EUR',20000.0,'20000','manual','exact','reviewed-receipt','{}','2020'),
 ('s','w','p','wallet','s','2024-03-01T00:00:00Z','outbound','BTC',150000000000,1000000,'sell','EUR',30000.0,'30000','manual','exact','reviewed-receipt','{}','2020'),
 ('i','w','p','wallet','i','2024-03-02T00:00:00Z','inbound','BTC',1000000000,0,'interest','EUR',30000.0,'30000','manual','exact','reviewed-receipt','{}','2020'),
 ('t','w','p','wallet','t','2024-04-01T00:00:00Z','outbound','BTC',20000000000,0,'sell','EUR',31000.5,'31000.5','manual','exact','reviewed-receipt','{}','2020');
INSERT OR REPLACE INTO settings VALUES('context_workspace','w'),('context_profile','p');
";

/// The book with its journals processed, as `_book` left it.
fn processed() -> Sandbox {
    let mut sandbox = Sandbox::book(BOOK);
    sandbox.journal_clock(CLOCK);
    let out = sandbox.machine(&["journals", "process"]);
    assert_eq!(out.code, 0, "{}", out.text());
    sandbox
}

fn journal_book() -> Sandbox {
    let sandbox = processed();
    sandbox.sql(
        "INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at)
          VALUES('acct','w','p','ACC','Treasury','asset','2020'),
          ('dup1','w','p','D1','Same','asset','2020'),('dup2','w','p','D2','Same','asset','2020');
         UPDATE journal_entries SET account_id='acct' WHERE transaction_id IN ('a','s');
         UPDATE journal_entries SET at_category='neu', at_kennzahl=801 WHERE transaction_id='b';
         UPDATE journal_entries SET calculation_json='{\"legacy\": true}', calculation_compact=NULL
          WHERE transaction_id='i';
         UPDATE journal_entries SET calculation_compact='[2, \"broken\"' WHERE transaction_id='t';",
    );
    sandbox
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn b64_decode(text: &str) -> Vec<u8> {
    let values: Vec<u32> = text
        .bytes()
        .map(|c| B64.iter().position(|b| *b == c).unwrap() as u32)
        .collect();
    let mut out = Vec::new();
    for chunk in values.chunks(4) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, v)| acc | v << (18 - 6 * i));
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    out
}

/// A cursor whose JSON payload `edit` changed, re-encoded as Python's
/// `json.dumps` spells it.
fn tamper(cursor: &str, edit: impl Fn(&mut serde_json::Map<String, serde_json::Value>)) -> String {
    let mut payload: serde_json::Value = serde_json::from_slice(&b64_decode(cursor)).unwrap();
    edit(payload.as_object_mut().unwrap());
    b64_encode(python_dumps(&payload).as_bytes())
}

/// `json.dumps` with its default separators.
fn python_dumps(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!(
                    "{}: {}",
                    serde_json::Value::from(k.as_str()),
                    python_dumps(v)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        serde_json::Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(python_dumps)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

#[test]
fn test_journal_entries_and_events_match_python_byte_for_byte() {
    let sandbox = journal_book();
    let before = sandbox.dump();
    let mut t = Transcript::new("cli_leaves/journal_entries_and_events");
    let total = sandbox
        .query_strings("SELECT id FROM journal_entries ORDER BY id")
        .len();
    let listed = t.machine(&sandbox, &["journals", "list"]).json();
    assert_eq!(listed["kind"], "journals.list");
    assert_eq!(listed["data"].as_array().unwrap().len(), total);
    for argv in [
        &["--limit", "0"][..],
        &["--limit", "-3"],
        &["--limit", "9223372036854775807"],
        &["--cursor", "bogus"],
        &["--cursor", "e30"],
        &["--workspace", "missing"],
        &["--profile", "missing"],
    ] {
        t.machine(&sandbox, &[&["journals", "list"][..], argv].concat());
    }
    let mut cursor: Option<String> = None;
    let mut seen = Vec::new();
    loop {
        let mut argv = vec!["journals", "list", "--limit", "1"];
        if let Some(c) = &cursor {
            argv.extend(["--cursor", c.as_str()]);
        }
        let page = t.machine(&sandbox, &argv).json();
        for entry in page["data"].as_array().unwrap() {
            seen.push(entry["id"].as_str().unwrap().to_owned());
        }
        match page["next_cursor"].as_str() {
            None => break,
            Some(next) => cursor = Some(next.to_owned()),
        }
        let c = cursor.clone().unwrap();
        let broken = [
            c[..c.len() - 3].to_owned(),
            tamper(&c, |p| {
                p["filters"]
                    .as_object_mut()
                    .unwrap()
                    .insert("asset".into(), "BTC".into());
            }),
            tamper(&c, |p| {
                p.remove("id");
            }),
            tamper(&c, |p| {
                p.insert("id".into(), serde_json::Value::Null);
            }),
            tamper(&c, |p| {
                p.insert("occurred_at".into(), 7.into());
            }),
        ];
        for b in &broken {
            t.machine(
                &sandbox,
                &["journals", "list", "--limit", "1", "--cursor", b],
            );
        }
    }
    assert_eq!(seen.len(), total);

    let events = t.machine(&sandbox, &["journals", "events", "list"]).json();
    assert_eq!(events["kind"], "journals.events.list");
    assert_eq!(events["data"]["events"].as_array().unwrap().len(), total);
    for argv in [
        &["--limit", "2"][..],
        &["--limit", "0"],
        &["--limit", "1000"],
        &["--limit", "1001"],
        &["--wallet", "wallet"],
        &["--wallet", " WALLET "],
        &["--wallet", "missing"],
        &["--wallet", ""],
        &["--account", "ACC"],
        &["--account", "treasury"],
        &["--account", "Same"],
        &["--account", "missing"],
        &["--asset", "btc"],
        &["--asset", "eth"],
        &["--entry-type", "DISPOSAL"],
        &["--entry-type", "Income"],
        &["--start", "2024-02-01T00:00:00Z"],
        &["--end", "2024-03-01"],
        &[
            "--start",
            "2024-02-01",
            "--end",
            "2024-03-01T23:59:59.5+01:00",
        ],
        &["--start", "yesterday"],
        &["--end", "2024-13-01"],
        &["--start", ""],
        &["--wallet", "missing", "--start", "bogus"],
        &["--limit", "0", "--start", "bogus"],
    ] {
        t.machine(
            &sandbox,
            &[&["journals", "events", "list"][..], argv].concat(),
        );
    }
    let mut cursor: Option<String> = None;
    let mut seen = 0;
    loop {
        let mut argv = vec![
            "journals", "events", "list", "--asset", "BTC", "--limit", "2",
        ];
        if let Some(c) = &cursor {
            argv.extend(["--cursor", c.as_str()]);
        }
        let page = t.machine(&sandbox, &argv).json();
        seen += page["data"]["events"].as_array().unwrap().len();
        match page["data"]["next_cursor"].as_str() {
            None => break,
            Some(next) => cursor = Some(next.to_owned()),
        }
        let c = cursor.clone().unwrap();
        t.machine(
            &sandbox,
            &["journals", "events", "list", "--limit", "2", "--cursor", &c],
        );
        t.machine(&sandbox, &["journals", "list", "--cursor", &c]);
    }
    assert_eq!(seen, total);

    let ids = sandbox.query_strings("SELECT id FROM journal_entries ORDER BY id");
    for id in &ids {
        let got = t
            .machine(&sandbox, &["journals", "events", "get", "--event-id", id])
            .json();
        assert_eq!(got["data"]["id"].as_str(), Some(id.as_str()));
        assert!(got["data"].get("calculation_compact").is_none());
    }
    for argv in [
        &["--event-id", "missing"][..],
        &["--event-id", ""],
        &["--event-id", "x", "--profile", "missing"],
    ] {
        t.machine(
            &sandbox,
            &[&["journals", "events", "get"][..], argv].concat(),
        );
    }
    for fmt in ["table", "plain", "csv", "json"] {
        t.run(
            &sandbox,
            &["--format", fmt, "journals", "list", "--limit", "2"],
        );
        t.run(
            &sandbox,
            &[
                "--format", fmt, "journals", "events", "list", "--limit", "2",
            ],
        );
        t.run(
            &sandbox,
            &[
                "--format",
                fmt,
                "journals",
                "events",
                "get",
                "--event-id",
                &ids[0],
            ],
        );
    }
    assert_eq!(sandbox.dump(), before, "reads leave the book untouched");
    t.finish();
}

fn transfer_book(country: &str) -> Sandbox {
    let sandbox = processed();
    let h = |c: char| c.to_string().repeat(64);
    sandbox.sql(&format!(
        "UPDATE profiles SET tax_country='{country}';
         INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at)
          VALUES('cold','w','p','Cold','manual','2020'),('liquid','w','p','Liquid','manual','2020');
         INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
          direction,asset,amount,fee,fiat_rate,raw_json,created_at) VALUES
          ('o1','w','p','wallet','ext-o1','fp-o1','2024-05-01T00:00:00Z','outbound','BTC',50000000,1000,30000.5,'{{}}','2020'),
          ('i1','w','p','cold','ext-i1','fp-i1','2024-05-01T00:00:00Z','inbound','BTC',30000000,0,NULL,'{{}}','2020'),
          ('i2','w','p','cold','ext-i2','fp-i2','2024-05-01T00:00:00Z','inbound','BTC',20000000,0,NULL,'{{}}','2020'),
          ('o2','w','p','wallet','ext-o2','fp-o2','2024-05-01T00:00:00Z','outbound','BTC',40000000,0,NULL,'{{}}','2020'),
          ('l2','w','p','liquid','ext-l2','fp-l2','2024-05-01T00:00:00Z','inbound','LBTC',39000000,0,NULL,'{{}}','2020'),
          ('o3','w','p','wallet','ext-o3','fp-o3','2024-05-01T00:00:00Z','outbound','BTC',10000000,0,NULL,'{{}}','2020');
         INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,
          target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,
          target_observation_hash,target_start_msat,target_end_msat,source_wallet_id,target_wallet_id,
          source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,
          basis_state,reason,atomic_group_id,component_id,occurred_at,created_at) VALUES
          ('{h1}','w','p','o1','i1','{ha}',0,30000000,'{hb}',0,30000000,'wallet','cold','main','main','bitcoin','bitcoin','BTC','BTC','internal_verified','eligible','move','group-1',NULL,'2024-05-01T00:00:00Z','2020'),
          ('{h2}','w','p','o1','i2','{ha}',30000000,50000000,'{hc}',0,20000000,'wallet',NULL,'main','main','bitcoin','bitcoin','BTC','BTC','internal_reviewed','blocked_by_prior_custody_basis','reviewed','','component-9','2024-05-01T00:00:00Z','2020'),
          ('{h3}','w','p','o2','l2','{hd}',0,39000000,'{he}',0,39000000,'wallet','liquid','main','main','bitcoin','bitcoin','BTC','LBTC','internal_verified','eligible','peg','',NULL,'2024-05-02T00:00:00Z','2020');
         INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,
          source_transaction_id,target_transaction_id,component_id,source_asset,target_asset,source_amount_msat,
          target_amount_msat,basis_state,occurred_at,target_occurred_at,created_at) VALUES
          ('{h4}','w','p','conversion','o2','l2',NULL,'BTC','LBTC',40000000,39000000,'eligible','2024-05-02T00:00:00Z',NULL,'2020'),
          ('{h5}','w','p','direct_payout','o3',NULL,NULL,'BTC','USDT',10000000,7000000000,'eligible','2024-05-03T00:00:00Z','2024-05-03T01:00:00Z','2020');
         INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,created_at)
          VALUES('o3','w','p','missing_spot_price','{{}}','2020');",
        h1 = h('1'),
        h2 = h('2'),
        h3 = h('3'),
        h4 = h('4'),
        h5 = h('5'),
        ha = h('a'),
        hb = h('b'),
        hc = h('c'),
        hd = h('d'),
        he = h('e'),
    ));
    sandbox
}

#[test]
fn test_transfer_audit_matches_python_byte_for_byte() {
    let sandbox = transfer_book("generic");
    let before = sandbox.dump();
    let mut t = Transcript::new("cli_leaves/transfer_audit");
    let audit = t
        .machine(&sandbox, &["journals", "transfers", "list"])
        .json();
    assert_eq!(audit["kind"], "journals.transfers.list");
    let data = &audit["data"];
    assert_eq!(
        data["summary"],
        serde_json::json!({"same_asset_transfers": 2, "custody_transfers": 3,
            "cross_asset_pairs": 1, "direct_swap_payouts": 1, "quarantines": 1})
    );
    let fees: Vec<i64> = data["same_asset_transfers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["fee_msat"].as_i64().unwrap())
        .collect();
    assert_eq!(fees, [1_000, 0], "the source fee is booked once");
    assert!(data["same_asset_transfers"][0]
        .get("transfer_group_id")
        .is_some());
    assert!(data["same_asset_transfers"][1]
        .get("transfer_group_id")
        .is_none());
    for argv in [&["--workspace", "missing"][..], &["--profile", "missing"]] {
        t.machine(
            &sandbox,
            &[&["journals", "transfers", "list"][..], argv].concat(),
        );
    }
    for fmt in ["table", "plain", "csv"] {
        t.run(
            &sandbox,
            &["--format", fmt, "journals", "transfers", "list"],
        );
    }
    assert_eq!(sandbox.dump(), before);
    t.finish();
}

#[test]
fn test_transfer_audit_tax_country_gate_matches_python() {
    let mut t = Transcript::new("cli_leaves/transfer_audit_tax_country_gate");
    for country in ["unsupported", "AT", ""] {
        let sandbox = transfer_book(country);
        t.note(&format!("tax_country={country:?}"));
        let out = t.machine(&sandbox, &["journals", "transfers", "list"]);
        if country == "unsupported" {
            assert_eq!(out.code, 1, "an unsupported country refuses the audit");
        }
    }
    t.finish();
}

const QUARANTINES: &str = "
INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,created_at) VALUES
 ('a','w','p','missing_spot_price','{}','2024-05-01T00:00:00Z'),
 ('b','w','p','transfer_pair_dependency_blocked','{\"blocked_by_transaction_ids\": [\"a\"]}','2024-05-01T00:00:00Z'),
 ('s','w','p','custody_basis_barrier','{\"root_transaction_ids\": [\"a\", \"missing\"]}','2024-05-01T00:00:00Z'),
 ('i','w','p','basis_provenance_incomplete','{}','2024-05-01T00:00:00Z'),
 ('t','w','p','insufficient_lots','{\"asset\": \"BTC\"}','2024-05-01T00:00:00Z');
";

fn held_book() -> Sandbox {
    let sandbox = processed();
    sandbox.sql(&format!(
        "UPDATE transactions SET external_id='ext-' || id;
         {QUARANTINES}
         UPDATE profiles SET fiat_currency='eur';"
    ));
    sandbox
}

#[test]
fn test_quarantine_reads_match_python_byte_for_byte() {
    let sandbox = held_book();
    let before = sandbox.dump();
    let mut t = Transcript::new("cli_leaves/quarantine_reads");
    let listed = t.machine(&sandbox, &["journals", "quarantined"]).json();
    let order: Vec<&str> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["transaction_id"].as_str().unwrap())
        .collect();
    assert_eq!(order, ["t", "i", "s", "b", "a"]);
    for row in listed["data"].as_array().unwrap() {
        for key in ["category", "root", "actions"] {
            assert!(row.get(key).is_some(), "{row}");
        }
    }
    for tx in ["a", "b", "s", "i", "t", "ext-a", "missing", ""] {
        t.machine(
            &sandbox,
            &["journals", "quarantine", "show", "--transaction", tx],
        );
    }
    t.machine(
        &sandbox,
        &["journals", "quarantined", "--profile", "missing"],
    );
    for fmt in ["table", "plain", "csv"] {
        t.run(&sandbox, &["--format", fmt, "journals", "quarantined"]);
        t.run(
            &sandbox,
            &[
                "--format",
                fmt,
                "journals",
                "quarantine",
                "show",
                "--transaction",
                "b",
            ],
        );
    }
    assert_eq!(sandbox.dump(), before);
    sandbox.sql("DELETE FROM journal_quarantines WHERE transaction_id='t'");
    t.machine(
        &sandbox,
        &["journals", "quarantine", "show", "--transaction", "t"],
    );
    t.finish();
}

#[test]
fn test_quarantine_reads_parse_stored_detail_as_python() {
    let mut t = Transcript::new("cli_leaves/quarantine_reads_parse_stored_detail");
    for detail in ["not json", "[1, 2]", "", "7"] {
        let sandbox = held_book();
        t.note(&format!("detail_json={detail:?}"));
        sandbox.sql(&format!(
            "UPDATE journal_quarantines SET detail_json='{detail}' WHERE transaction_id='i'"
        ));
        t.machine(&sandbox, &["journals", "quarantined"]);
        t.machine(
            &sandbox,
            &["journals", "quarantine", "show", "--transaction", "i"],
        );
    }
    t.finish();
}

#[test]
fn test_quarantine_clear_matches_python_envelope_and_book() {
    let mut t = Transcript::new("cli_leaves/quarantine_clear");
    for argv in [
        &["--transaction", "a"][..],
        &["--transaction", "ext-b"],
        &["--transaction", "missing"],
        &["--transaction", "a", "--profile", "missing"],
    ] {
        let sandbox = held_book();
        let before = sandbox.dump();
        let leaf = [&["journals", "quarantine", "clear"][..], argv].concat();
        t.machine(&sandbox, &leaf);
        t.changes(&sandbox, &before, "after the clear");
        let before = sandbox.dump();
        // A second clear finds nothing held.
        t.machine(&sandbox, &leaf);
        t.changes(&sandbox, &before, "after the second clear");
    }
    t.finish();
}

#[test]
fn test_quarantine_clear_refuses_a_row_that_is_not_held() {
    let sandbox = held_book();
    sandbox.sql("DELETE FROM journal_quarantines WHERE transaction_id='a'");
    let before = sandbox.dump();
    let mut t = Transcript::new("cli_leaves/quarantine_clear_refuses_unheld");
    let out = t.machine(
        &sandbox,
        &["journals", "quarantine", "clear", "--transaction", "a"],
    );
    assert_eq!(out.json()["error"]["code"], "not_found");
    assert_eq!(sandbox.dump(), before);
    t.finish();
}

const RESOLVES: &[&[&str]] = &[
    &[
        "price-override",
        "--transaction",
        "a",
        "--fiat-rate",
        "41234.5",
    ],
    &[
        "price-override",
        "--transaction",
        "ext-a",
        "--fiat-value",
        "4123.45678901234567890123456789",
    ],
    &[
        "price-override",
        "--transaction",
        "a",
        "--fiat-rate",
        "10",
        "--fiat-value",
        "2",
    ],
    &[
        "price-override",
        "--transaction",
        "a",
        "--fiat-rate",
        "1E+3",
        "--fiat-value",
        "0",
    ],
    &["exclude", "--transaction", "t"],
    &[
        "price-override",
        "--group-id",
        "root:a",
        "--fiat-rate",
        "10000",
    ],
    &[
        "price-override",
        "--group-id",
        "root:a",
        "--fiat-value",
        "100",
        "--reprocess",
    ],
    &["exclude", "--group-id", "root:t"],
    &["exclude", "--transaction", "t", "--reprocess"],
    &["exclude", "--transaction", "i", "--reprocess"],
];

#[test]
fn test_quarantine_resolve_matches_python_envelope_and_book() {
    let mut t = Transcript::new("cli_leaves/quarantine_resolve");
    for argv in RESOLVES {
        let sandbox = held_book();
        let before = sandbox.dump();
        let leaf = [&["journals", "quarantine", "resolve"][..], argv].concat();
        let got = t.machine(&sandbox, &leaf).json();
        assert_eq!(
            got["kind"].as_str(),
            Some(format!("journals.quarantine.resolve.{}", argv[0]).as_str()),
            "{got}"
        );
        t.changes(&sandbox, &before, "after the resolution");
    }
    t.finish();
}

const REFUSED: &[&[&str]] = &[
    &["price-override", "--transaction", "a"],
    &["price-override", "--transaction", "a", "--fiat-rate", "abc"],
    &["price-override", "--transaction", "a", "--fiat-rate", "-1"],
    &["price-override", "--transaction", "a", "--fiat-rate", "0"],
    &["price-override", "--transaction", "a", "--fiat-rate", ""],
    &["price-override", "--transaction", "a", "--fiat-rate", "NaN"],
    &[
        "price-override",
        "--transaction",
        "a",
        "--fiat-value",
        "Infinity",
    ],
    &["price-override", "--transaction", "a", "--fiat-value", "-5"],
    &[
        "price-override",
        "--transaction",
        "a",
        "--fiat-rate",
        "1",
        "--fiat-value",
        "-5",
    ],
    &[
        "price-override",
        "--transaction",
        "a",
        "--fiat-rate",
        "-1",
        "--fiat-value",
        "abc",
    ],
    &[
        "price-override",
        "--transaction",
        "a",
        "--fiat-rate",
        "abc",
        "--fiat-value",
        "-1",
    ],
    &[
        "price-override",
        "--transaction",
        "a",
        "--fiat-rate",
        "1",
        "--fiat-value",
        "NaN",
    ],
    &["price-override", "--group-id", "root:b", "--fiat-rate", "1"],
    &[
        "price-override",
        "--group-id",
        "reason:missing_spot_price",
        "--fiat-rate",
        "1",
    ],
    &["exclude", "--group-id", ""],
    &["exclude", "--transaction", "missing"],
    &["exclude", "--transaction", "missing", "--reprocess"],
    &["exclude", "--transaction", "a", "--workspace", "missing"],
    &["exclude", "--transaction", "a", "--fiat-rate", "1"],
    &["exclude"],
    &["exclude", "--transaction", "a", "--group-id", "root:a"],
];

#[test]
fn test_quarantine_resolve_refusals_match_python_without_writes() {
    let sandbox = held_book();
    let before = sandbox.dump();
    let mut t = Transcript::new("cli_leaves/quarantine_resolve_refusals");
    for argv in REFUSED {
        let out = t.machine(
            &sandbox,
            &[&["journals", "quarantine", "resolve"][..], argv].concat(),
        );
        assert_ne!(out.code, 0, "{argv:?} is refused");
    }
    assert_eq!(sandbox.dump(), before);
    t.finish();
}

#[test]
fn test_quarantine_resolve_human_formats_match_python() {
    let mut t = Transcript::new("cli_leaves/quarantine_resolve_human_formats");
    for fmt in ["table", "plain", "csv"] {
        let sandbox = held_book();
        t.run(
            &sandbox,
            &[
                "--format",
                fmt,
                "journals",
                "quarantine",
                "resolve",
                "exclude",
                "--transaction",
                "t",
            ],
        );
    }
    t.finish();
}

fn wallet_book() -> Sandbox {
    let sandbox = processed();
    let change = DESCRIPTOR.replace("/0/*)", "/1/*)");
    sandbox.sql(&format!(
        "INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at)
          VALUES('acct','w','p','ACC','Treasury','asset','2020');
         INSERT INTO wallets(id,workspace_id,profile_id,account_id,label,kind,config_json,created_at) VALUES
          ('desc','w','p','acct','Descriptor','descriptor','{{\"descriptor\": \"{DESCRIPTOR}\", \"change_descriptor\": \"{change}\", \"network\": \"bitcoin\", \"blinding_key\": null, \"gap_limit\": 5}}','2020'),
          ('list','w','p',NULL,'Watch list','address','{{\"addresses\": [\"{LISTED}\"], \"chain\": \"bitcoin\"}}','2020'),
          ('sp','w','p',NULL,'Silent','silent-payment','{{\"sp_descriptor\": \"  sp(raw-test)  \"}}','2020'),
          ('odd','w','p',NULL,'Odd','manual','[1, 2]','2020'),
          ('twin1','w','p',NULL,'Twin','manual','','2020'),
          ('twin2','w','p',NULL,'twin','manual','null','2020');
         INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at)
          VALUES('core','bitcoinrpc','bitcoin','main','http://127.0.0.1:1','2025','2025');"
    ));
    sandbox
}

#[test]
fn test_reveal_descriptor_matches_python_and_keeps_its_gates() {
    let sandbox = wallet_book();
    let before = sandbox.dump();
    let change = DESCRIPTOR.replace("/0/*)", "/1/*)");
    let mut t = Transcript::new("cli_leaves/reveal_descriptor");
    for wallet in [
        "desc",
        "Descriptor",
        "Watch list",
        "sp",
        "wallet",
        "odd",
        "Twin",
        "missing",
        "",
    ] {
        for extra in [&[][..], &["--material-only"]] {
            t.machine(
                &sandbox,
                &[
                    &["wallets", "reveal-descriptor", "--wallet", wallet][..],
                    extra,
                ]
                .concat(),
            );
        }
    }
    let revealed = sandbox
        .machine(&["wallets", "reveal-descriptor", "--wallet", "desc"])
        .json();
    assert_eq!(revealed["data"]["descriptor"], DESCRIPTOR);
    assert_eq!(
        revealed["data"]["wallet_material"].as_str().unwrap(),
        format!("{DESCRIPTOR}\n{change}")
    );
    let material = sandbox.machine(&[
        "wallets",
        "reveal-descriptor",
        "--wallet",
        "desc",
        "--material-only",
    ]);
    assert_eq!(material.text(), format!("{DESCRIPTOR}\n{change}\n"));
    t.machine(
        &sandbox,
        &[
            "wallets",
            "reveal-descriptor",
            "--wallet",
            "desc",
            "--profile",
            "missing",
        ],
    );
    for fmt in ["table", "plain", "csv", "json"] {
        t.run(
            &sandbox,
            &[
                "--format",
                fmt,
                "wallets",
                "reveal-descriptor",
                "--wallet",
                "desc",
            ],
        );
    }
    let target = sandbox.path("material.txt");
    let target_arg = target.to_string_lossy().into_owned();
    let out = t.machine(
        &sandbox,
        &[
            "--output",
            &target_arg,
            "wallets",
            "reveal-descriptor",
            "--wallet",
            "desc",
            "--material-only",
        ],
    );
    assert_eq!((out.code, out.stdout.as_slice()), (0, &b""[..]));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        format!("{DESCRIPTOR}\n{change}\n")
    );
    assert_eq!(sandbox.dump(), before);
    t.finish();
}

#[test]
fn test_identify_matches_python_in_every_format() {
    let sandbox = wallet_book();
    let mut t = Transcript::new("cli_leaves/identify");
    // A wallet whose stored configuration is not an object fails the scan.
    for wallet in ["odd", "twin2"] {
        t.machine(&sandbox, &["wallets", "identify", "--address", LISTED]);
        sandbox.sql(&format!("DELETE FROM wallets WHERE id='{wallet}'"));
    }
    let owned = t
        .machine(&sandbox, &["wallets", "identify", "--address", LISTED])
        .json();
    assert_eq!(owned["data"]["results"][0]["status"], "owned", "{owned}");
    let before = sandbox.dump();
    let f64 = "f".repeat(64);
    let candidates = sandbox.path("candidates.txt");
    std::fs::write(
        &candidates,
        format!("# owned?\r\n{LISTED}\r\n{f64}\rbc1qnope\n"),
    )
    .unwrap();
    let sheet = sandbox.path("sheet.csv");
    std::fs::write(
        &sheet,
        format!(
            "date,address,txid\n2024-01-01,{LISTED},{}\n2024-01-02,BC1QNOPE,\n",
            "e".repeat(64)
        ),
    )
    .unwrap();
    let binary = sandbox.path("binary.txt");
    std::fs::write(&binary, b"bc1q\x00").unwrap();
    let latin = sandbox.path("latin.txt");
    std::fs::write(&latin, b"caf\xe9").unwrap();
    let p = |path: &std::path::Path| path.to_string_lossy().into_owned();
    let (candidates, sheet, binary, latin) = (p(&candidates), p(&sheet), p(&binary), p(&latin));
    let missing_txt = p(&sandbox.path("missing.txt"));
    let missing_csv = p(&sandbox.path("missing.csv"));
    let base = p(&sandbox.base);
    let upper = LISTED.to_uppercase();
    let cases: Vec<Vec<&str>> = vec![
        vec![
            "--address",
            LISTED,
            "--txid",
            &f64,
            "--candidate",
            "lnbc1bogus",
            "--candidate",
            "xpub-nope",
        ],
        vec!["--address", &upper, "--wallet", "Watch list"],
        vec!["--address", LISTED, "--wallet", "desc"],
        vec!["--address", LISTED, "--wallet", "missing"],
        vec!["--address", LISTED, "--wallet", "Twin"],
        vec!["--file", &candidates],
        vec!["--csv", &sheet],
        vec!["--file", &candidates, "--csv", &sheet],
        vec!["--file", &missing_txt],
        vec!["--csv", &missing_csv],
        vec!["--file", &base],
        vec!["--file", &binary],
        vec!["--file", &latin],
        vec!["--file", ""],
        vec!["--address", LISTED, "--scan-to-index", "0"],
        vec!["--address", LISTED, "--scan-to-index", "-1"],
        vec![
            "--address",
            LISTED,
            "--scan-to-index",
            "99999999999999999999",
        ],
        vec![],
        vec!["--scan-to-index", "-1"],
        vec!["--address", ""],
        vec![
            "--txid",
            &f64,
            "--verify-on-chain",
            "--verify-backend",
            "core",
        ],
        vec![
            "--txid",
            &f64,
            "--verify-on-chain",
            "--verify-backend",
            "missing",
        ],
        vec!["--address", LISTED, "--profile", "missing"],
    ];
    for argv in &cases {
        t.machine(&sandbox, &[&["wallets", "identify"][..], argv].concat());
    }
    for fmt in ["table", "plain", "csv", "json"] {
        t.run(
            &sandbox,
            &[
                "--format",
                fmt,
                "wallets",
                "identify",
                "--address",
                LISTED,
                "--txid",
                &f64,
                "--candidate",
                "nonsense",
            ],
        );
    }
    assert_eq!(sandbox.dump(), before);
    t.finish();
}

#[test]
fn test_identify_verify_on_chain_without_pending_txids_reaches_no_backend() {
    let sandbox = wallet_book();
    sandbox.sql(
        "DELETE FROM wallets WHERE id IN ('odd', 'twin2');
         INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at)
          VALUES('esp','esplora','bitcoin','main','http://127.0.0.1:9','2025','2025');",
    );
    let mut t = Transcript::new("cli_leaves/identify_verify_on_chain_local");
    let got = t
        .machine(
            &sandbox,
            &[
                "wallets",
                "identify",
                "--address",
                LISTED,
                "--verify-on-chain",
                "--verify-backend",
                "esp",
            ],
        )
        .json();
    assert_eq!(got["data"]["summary"]["verified_on_chain"], true);
    t.finish();
}

const ANALYZED: &str =
    "Datum;Richtung;Stueck;Kurs\n2024-01-02;Kauf;0,5;40000\n2024-02-03;Verkauf;0,1;45000\n";
const LEDGER: &str = "Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fee Amount,Fee Asset,Fiat Value,Counterparty,Note,Tx-ID\n\
Buy,2026-01-15,0.05000000,BTC,3000.00,EUR,3.50,EUR,,Coinfinity,First stack,\n\
Sell,2026-02-10,2200.00,EUR,0.03000000,BTC,1.00,EUR,,Kraken,Took some profit,\n\
Mining,2026-03-10,0.00050000,BTC,,,,,32.50,Solo pool,Block reward,\n";

/// `wallets analyze-file` from a work directory, on a data root that must
/// stay a layout without a book.
fn analyze(t: &mut Transcript, sandbox: &Sandbox, options: &[&str], argv: &[&str]) -> i32 {
    let work = sandbox.path("work");
    if !work.exists() {
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("export.csv"), ANALYZED).unwrap();
        std::fs::write(
            work.join("map.json"),
            r#"{"date":"Datum","type":"Richtung","amount":"Stueck","price":"Kurs"}"#,
        )
        .unwrap();
        std::fs::write(work.join("ledger.csv"), LEDGER).unwrap();
        std::fs::write(work.join("scan.pdf"), b"%PDF-1.4\n").unwrap();
        std::fs::write(work.join("notes.docx"), b"PK").unwrap();
    }
    let root = sandbox.path("analyze-root");
    let mut args: Vec<String> = options.iter().map(|s| (*s).to_owned()).collect();
    args.extend([
        "--data-root".to_owned(),
        root.to_string_lossy().into_owned(),
        "wallets".to_owned(),
        "analyze-file".to_owned(),
    ]);
    args.extend(argv.iter().map(|s| (*s).to_owned()));
    let mut command = sandbox.command();
    command.args(&args).current_dir(&work);
    let output = command.output().unwrap();
    let output = crate::support::Output {
        code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    };
    let shown: Vec<&str> = args.iter().map(String::as_str).collect();
    t.output(sandbox, &shown, &output);
    assert!(
        !root.join("kassiber.sqlite3").exists(),
        "a layout, never a book"
    );
    if output.code == 0 && options == ["--machine"] {
        assert_eq!(output.json()["kind"], "wallets.file_analysis");
    }
    output.code
}

#[test]
fn test_analyze_file_matches_python_without_a_book() {
    let sandbox = Sandbox::empty();
    let mut t = Transcript::new("cli_leaves/analyze_file");
    for argv in [
        &[][..],
        &["--sample-rows", "0"],
        &["--sample-rows", "-4"],
        &["--sample-rows", "1000"],
        &["--file", "ledger.csv", "--sample-rows", "-4"],
        &["--file", "ledger.csv", "--sample-rows", "2"],
        &[
            "--column-map",
            r#"{"date":"Datum","type":"Richtung","amount":"Stueck"}"#,
        ],
        &["--column-map", "@map.json"],
        &["--column-map", "@missing.json"],
        &["--column-map", "not json"],
        &["--column-map", "[1]"],
        &["--column-map", "  "],
    ] {
        analyze(
            &mut t,
            &sandbox,
            &["--machine"],
            &[&["--file", "export.csv"][..], argv].concat(),
        );
    }
    t.finish();
}

#[test]
fn test_analyze_file_refusals_and_formats_match_python() {
    let sandbox = Sandbox::empty();
    let mut t = Transcript::new("cli_leaves/analyze_file_refusals_and_formats");
    for argv in [
        ["--file", "export.csv"],
        ["--file", "scan.pdf"],
        ["--file", "notes.docx"],
        ["--file", "missing.csv"],
        ["--file", "."],
    ] {
        for options in [
            &["--machine"][..],
            &["--format", "table"],
            &["--format", "plain"],
        ] {
            analyze(&mut t, &sandbox, options, &argv);
        }
    }
    t.finish();
}

#[test]
fn test_non_object_cursor_refuses_where_python_crashes() {
    let sandbox = journal_book();
    for payload in [
        "5",
        "null",
        r#"["filters", "occurred_at", "created_at", "id"]"#,
    ] {
        let cursor = b64_encode(payload.as_bytes());
        for leaf in [&["journals", "list"][..], &["journals", "events", "list"]] {
            let out = sandbox.machine(&[leaf, &["--cursor", &cursor]].concat());
            assert_eq!(
                out.json()["error"],
                serde_json::json!({
                    "code": "validation",
                    "message": format!("Invalid cursor: {cursor}"),
                    "hint": "Pass the exact next_cursor value from the previous response; do not modify it or change filters.",
                    "details": null,
                    "retryable": false,
                    "debug": null,
                })
            );
        }
    }
}

/// Populated archives with the guards Kassiber shipped for them, plus one
/// empty archive table.
fn ledger_book(extra_sql: &str) -> Sandbox {
    let sandbox = processed();
    let triggers: serde_json::Value = serde_json::from_str(include_str!(
        "../../../store/fixtures/legacy-ledger-triggers.json"
    ))
    .unwrap();
    let tables = [
        "gl_books",
        "gl_entries",
        "gl_lines",
        "gl_evidence",
        "external_documents",
    ];
    let mut sql = String::from(
        "CREATE TABLE gl_books(profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE RESTRICT,
          currency TEXT NOT NULL, created_at TEXT NOT NULL);
         CREATE TABLE gl_entries(id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profiles(id)
          ON DELETE RESTRICT, description TEXT NOT NULL, UNIQUE(profile_id, id));
         CREATE TABLE gl_lines(entry_id TEXT NOT NULL, profile_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
          PRIMARY KEY(entry_id, ordinal));
         CREATE TABLE gl_evidence(id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, content BLOB NOT NULL,
          source_document_id TEXT REFERENCES external_documents(id) ON DELETE RESTRICT);
         CREATE TABLE gl_empty(id TEXT);
         INSERT INTO external_documents(id, workspace_id, profile_id, document_type, label, created_at,
          updated_at) VALUES ('doc-1', 'w', 'p', 'invoice', 'Invoice', '2026-01-01T00:00:00Z',
          '2026-01-01T00:00:00Z');
         INSERT INTO gl_books VALUES ('p', 'EUR', '2026-01-01T00:00:00Z');
         INSERT INTO gl_entries VALUES ('entry-1', 'p', 'Opening'), ('entry-2', 'p', 'Close');
         INSERT INTO gl_lines VALUES ('entry-1', 'p', 1);
         INSERT INTO gl_evidence VALUES ('ev-1', 'p', x'00', 'doc-1');\n",
    );
    for trigger in triggers["triggers"].as_array().unwrap() {
        if tables.contains(&trigger["table"].as_str().unwrap()) {
            sql.push_str(trigger["sql"].as_str().unwrap());
            sql.push_str(";\n");
        }
    }
    sql.push_str(extra_sql);
    sandbox.sql(&sql);
    sandbox
}

#[test]
fn test_purge_legacy_ledger_matches_python_envelope_and_book() {
    let sandbox = ledger_book("");
    let before = sandbox.dump();
    let mut t = Transcript::new("cli_leaves/purge_legacy_ledger");
    let plan = t
        .machine(&sandbox, &["maintenance", "purge-legacy-ledger"])
        .json();
    assert_eq!(plan["kind"], "maintenance.purge-legacy-ledger.plan");
    assert_eq!(
        plan["data"]["tables"],
        serde_json::json!([
            {"table": "gl_books", "rows": 1}, {"table": "gl_entries", "rows": 2},
            {"table": "gl_evidence", "rows": 1}, {"table": "gl_lines", "rows": 1},
        ])
    );
    assert_eq!(plan["data"]["applied"], false);
    assert_eq!(plan["data"]["table_count"], 5);
    assert!(plan["data"]["trigger_count"].as_i64().unwrap() > 3);
    assert_eq!(sandbox.dump(), before);
    for fmt in ["table", "plain"] {
        t.run(
            &sandbox,
            &["--format", fmt, "maintenance", "purge-legacy-ledger"],
        );
    }
    let done = t
        .machine(
            &sandbox,
            &["maintenance", "purge-legacy-ledger", "--confirm"],
        )
        .json();
    assert_eq!(done["kind"], "maintenance.purge-legacy-ledger");
    assert_eq!(done["data"]["applied"], true);
    t.changes(&sandbox, &before, "after the purge");
    assert!(!sandbox.dump().contains("[gl_"));
    t.machine(&sandbox, &["maintenance", "purge-legacy-ledger"]);
    t.machine(
        &sandbox,
        &["maintenance", "purge-legacy-ledger", "--confirm"],
    );
    t.finish();
}

#[test]
fn test_unshipped_ledger_trigger_refuses_where_python_opens() {
    for trigger in [
        "CREATE TRIGGER gl_evidence_document_mirror AFTER INSERT ON external_documents \
         BEGIN UPDATE profiles SET label = NEW.label; END;",
        "CREATE TRIGGER gl_evidence_guard BEFORE DELETE ON gl_evidence BEGIN SELECT 1; END;",
    ] {
        let sandbox = ledger_book(trigger);
        let before = sandbox.dump();
        let out = sandbox.machine(&["maintenance", "purge-legacy-ledger", "--confirm"]);
        assert_eq!(out.code, 1);
        assert_eq!(out.json()["error"]["code"], "requires_python_migration");
        assert_eq!(sandbox.dump(), before);
    }
}
