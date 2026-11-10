//! Desktop freshness, maintenance, backend, Lightning and UTXO kinds served
//! by the daemon, against frozen goldens (see `support`). These replaced
//! `tests/test_rust_desktop_{settings,backends,lightning_test,utxos}.py`,
//! which compared the same sessions with the Python daemon.

#![cfg(feature = "test-harness")]

mod support;

use kassiber_envelope::get;
use kassiber_pyjson::Value;
use support::{kind_of, list, path, text, Desk};

const FRESHNESS: &str = r#"
INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,state,stale_reason,
  blocking_reports,paused,rate_limited_until,cooldown_reason,retry_count,last_success_at,last_error_at,last_error_code,
  last_error_message,last_phase,progress_json,checkpoint_json,updated_at) VALUES
 ('p','onchain_wallet:cold','onchain_wallet','Cold','error','partially_stale','fetch_failed',1,0,NULL,NULL,2,
  '2025-01-01T00:00:00Z','2025-01-02T00:00:00Z','backend_unreachable',
  'GET https://user:pw@esplora.example/api/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0 failed: host=''10.0.0.9'' token=abc.',
  'backend_fetch','{"phase":"backend_fetch","backend_url":"http://secret.example","token":"t","nested":{"descriptor":"wpkh(x)","xpub":""}}',
  '{"tip":"secret-checkpoint"}','2025-01-02T00:00:00Z'),
 ('p','market_rates:p','market_rates','Rates','rate_limited','fresh',NULL,0,1,'2030-01-01T00:00:00Z','http_429',0,
  NULL,NULL,NULL,NULL,NULL,'{}','{}','2025-01-03T00:00:00Z'),
 ('foreign','onchain_wallet:foreign','onchain_wallet','Foreign','error','stale',NULL,1,0,NULL,NULL,0,NULL,NULL,NULL,
  'private foreign failure',NULL,'{}','{}','2025');
INSERT INTO freshness_jobs(id,profile_id,job_type,source_key,source_type,source_label,status,phase,priority,payload_json,
  progress_json,result_json,error_json,attempts,cancel_requested,run_after,cooldown_until,created_at,started_at,finished_at,updated_at)
VALUES
 ('j1','p','onchain_wallet_history','onchain_wallet:cold','onchain_wallet','Cold','queued',NULL,50,
  '{"wallet_id":"cold","wallet_label":"Cold","url":"tcp://electrum.example:50001","api_key":"k","checkpoint":{"x":1}}',
  '{}','{}','{"message":"see ssl://node.example:50002!"}',0,0,NULL,NULL,'2025-01-01T00:00:00Z',NULL,NULL,'2025'),
 ('j2','p','market_rate_coverage','market_rates:p','market_rates','Rates','rate_limited','rate_coverage',100,'not json',
  '{}','{}','{}',1,1,NULL,'2030-01-01T00:00:00Z','2025-01-02T00:00:00Z',NULL,NULL,'2025'),
 ('j3','p','journal_refresh','journals:p','journals','Journals','running','journal_refresh',10,'{}','{"phase":"x"}',
  '{}','{}',1,0,NULL,NULL,'2025-01-03T00:00:00Z','2025',NULL,'2025'),
 ('j4','p','onchain_wallet_history','onchain_wallet:cold','onchain_wallet','Cold','done',NULL,50,'{}','{}','{}','{}',
  1,0,NULL,NULL,'2024',NULL,NULL,'2024'),
 ('j5','foreign','journal_refresh','journals:foreign','journals','Journals','queued',NULL,10,'{}','{}','{}','{}',
  0,0,NULL,NULL,'2025',NULL,NULL,'2025');
UPDATE transactions SET pricing_quality='coarse_fallback' WHERE id IN ('a','b','g');
UPDATE transactions SET excluded=1 WHERE id='g';
"#;

const NO_CONTEXT: &str =
    "DELETE FROM settings WHERE key IN ('context_workspace','context_profile');";

const POLICIES: &[&str] = &[
    "",
    "INSERT INTO settings(key,value) VALUES('ai.auto_sync_before_report_reads.profile.p',' Yes ');",
    "INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p','[1]');",
    "INSERT OR REPLACE INTO settings(key,value) VALUES('freshness.policy.profile.p',\
     '{\"background_enabled\":1,\"source_classes\":{\"market_rates\":\"x\",\"bogus\":true,\"journals\":0}}');",
    "INSERT OR REPLACE INTO settings(key,value) VALUES('market_rate_provider',' MEMPOOL ');",
    "INSERT OR REPLACE INTO settings(key,value) VALUES('market_rate_provider','kraken-csv');",
    "UPDATE profiles SET fiat_currency='usd ', require_coarse_review=1, bitcoin_rail_carrying_value=0 WHERE id='p';",
    "UPDATE profiles SET fiat_currency='CHF' WHERE id='p';",
];

const READ_ARGS: &[Option<&str>] = &[
    None,
    Some("null"),
    Some("[]"),
    Some("\"x\""),
    Some("3"),
    Some("{}"),
    Some("{\"unexpected\": true}"),
];

const CONFIGURE_REFUSALS: &[Option<&str>] = &[
    Some("[]"),
    Some("\"x\""),
    Some(r#"{"bogus": 1, "also": 2, "source_classes": {}}"#),
    Some(r#"{"source_classes": []}"#),
    Some(r#"{"source_classes": "x"}"#),
    Some(r#"{"market_rate_provider": 3}"#),
    Some(r#"{"market_rate_provider": ["a"]}"#),
    Some(r#"{"auto_sync_before_report_reads": 1}"#),
    Some(r#"{"auto_sync_before_report_reads": "yes"}"#),
    Some(r#"{"market_rate_provider": "kraken-csv"}"#),
    Some(r#"{"market_rate_provider": "\ud800"}"#),
    Some(r#"{"source_classes": {"onchain_wallet": true, "bogus": true}}"#),
    Some(r#"{"market_rate_provider": "mempool", "source_classes": {"bogus": 1}}"#),
];

const CONFIGURE_WRITES: &[Option<&str>] = &[
    None,
    Some("null"),
    Some("{}"),
    Some(r#"{"background_enabled": 1}"#),
    Some(r#"{"background_enabled": null, "report_read_sync": "x"}"#),
    Some(r#"{"source_classes": {"market_rates": 1, "journals": 0}}"#),
    Some(r#"{"source_classes": null}"#),
    Some(r#"{"market_rate_provider": " CoinGecko\t"}"#),
    Some(r#"{"market_rate_provider": ""}"#),
    Some(
        r#"{"auto_sync_before_report_reads": true, "report_read_sync": false, "source_classes": {"journals": true}}"#,
    ),
    Some(r#"{"auto_sync_before_report_reads": false}"#),
    Some(r#"{"auto_sync_before_report_reads": null, "report_read_sync": 0}"#),
];

const STATUS: &str = "ui.freshness.status";
const MAINTENANCE: &str = "ui.maintenance.settings";

fn settings_desk(golden: &str) -> Desk {
    let desk = Desk::new(golden);
    desk.change(FRESHNESS);
    desk
}

#[test]
fn settings_status_reads_match_wire_bytes_across_policies() {
    let mut desk = settings_desk("settings_status_reads_match_wire_bytes_across_policies");
    let mut responses = Vec::new();
    for sql in POLICIES {
        desk.change(sql);
        responses = desk.session(STATUS, READ_ARGS, false);
        assert!(responses.iter().all(|r| kind_of(r) == STATUS));
        desk.session(MAINTENANCE, READ_ARGS, false);
    }
    let keys: Vec<&str> = list(&responses[0], &["data", "sources"])
        .iter()
        .map(|s| path(s, &["source_key"]).and_then(text).unwrap())
        .collect();
    assert_eq!(keys, ["market_rates:p", "onchain_wallet:cold"]);
    desk.check();
}

#[test]
fn settings_status_without_or_with_mismatched_context() {
    let mut desk = settings_desk("settings_status_without_or_with_mismatched_context");
    desk.change("UPDATE settings SET value='foreign' WHERE key='context_profile';");
    desk.session(MAINTENANCE, &[Some("{}")], false);
    desk.session(STATUS, &[Some("{}")], false);
    desk.change(
        "INSERT INTO workspaces(id,label,created_at) VALUES('w2','','2025');\
         UPDATE settings SET value='w2' WHERE key='context_workspace';",
    );
    desk.session(MAINTENANCE, &[Some("{}")], false);
    desk.session(STATUS, &[Some("{}")], false);
    desk.change(NO_CONTEXT);
    for kind in [
        STATUS,
        MAINTENANCE,
        "ui.freshness.configure",
        "ui.maintenance.configure",
    ] {
        desk.session(
            kind,
            &[
                Some("{}"),
                Some(r#"{"bogus": 1}"#),
                Some(r#"{"require_coarse_review": 3}"#),
            ],
            false,
        );
    }
    desk.check();
}

#[test]
fn settings_configure_refusals_leave_every_table_unchanged() {
    let mut desk = settings_desk("settings_configure_refusals_leave_every_table_unchanged");
    for kind in ["ui.freshness.configure", "ui.maintenance.configure"] {
        let responses = desk.session(kind, CONFIGURE_REFUSALS, false);
        assert!(responses.iter().all(|r| kind_of(r) == "error"));
    }
    desk.session(
        "ui.maintenance.configure",
        &[
            Some(r#"{"require_coarse_review": 1}"#),
            Some(r#"{"bitcoin_rail_carrying_value": "no"}"#),
            Some(r#"{"require_coarse_review": true, "bitcoin_rail_carrying_value": 0}"#),
            Some(r#"{"require_coarse_review": null, "source_classes": 1}"#),
        ],
        false,
    );
    desk.check();
}

#[test]
fn settings_configure_writes_match_bytes_and_tables() {
    let mut desk = settings_desk("settings_configure_writes_match_bytes_and_tables");
    for kind in ["ui.freshness.configure", "ui.maintenance.configure"] {
        let responses = desk.session(kind, CONFIGURE_WRITES, true);
        assert!(responses.iter().all(|r| kind_of(r) == kind));
        desk.session(STATUS, &READ_ARGS[..2], false);
    }
    desk.check();
}

#[test]
fn settings_maintenance_profile_flags_commit_before_policy_validation() {
    let mut desk =
        settings_desk("settings_maintenance_profile_flags_commit_before_policy_validation");
    desk.session(
        "ui.maintenance.configure",
        &[
            Some(r#"{"require_coarse_review": true, "bitcoin_rail_carrying_value": false}"#),
            Some(r#"{"require_coarse_review": true}"#),
            Some(r#"{"require_coarse_review": false, "market_rate_provider": "bogus"}"#),
            Some(r#"{"bitcoin_rail_carrying_value": true, "source_classes": {"bogus": true}}"#),
            Some(
                r#"{"bitcoin_rail_carrying_value": false, "auto_sync_before_report_reads": true, "market_rate_provider": "mempool"}"#,
            ),
        ],
        true,
    );
    desk.check();
}

// --- ui.backends.list (tests/test_rust_desktop_backends.py) ---

const BACKENDS: &[&str] = &[
    r#"{"name": "node", "kind": "lnd", "url": "https://user:pw@lnd.example:8080/v1?macaroon=abcd", "token": "abcd"}"#,
    r#"{"name": "cln", "kind": "coreln", "url": "https://cln.example:3010", "token": "synthetic-rune"}"#,
    r#"{"name": "core", "kind": "bitcoinrpc", "url": "http://127.0.0.1:18443", "config": {"username": "synthetic-user", "password": "synthetic-password"}}"#,
    r#"{"name": "electrum1", "kind": "electrum", "url": "ssl://electrum.example:50002", "config": {"certificate": "/synthetic/ca.pem", "insecure": "yes"}}"#,
    r#"{"name": "btcpay1", "kind": "btcpay", "url": "https://pay.example", "token": "synthetic-api-key"}"#,
    r#"{"name": "unused", "kind": "esplora", "url": "https://unused.example/api"}"#,
    r#"{"name": "bare", "kind": "esplora", "url": "bare.example:3000"}"#,
];

const BACKEND_WALLETS: &str = r#"
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('w-node','w','p','Node','lnd','{"backend":" node "}','2025'),
 ('w-cln','w','p','CLN','coreln','{"backend":"cln"}','2025'),
 ('w-core','w','p','Core','descriptor','{"backend":"core","descriptor":"wpkh(synthetic)"}','2025'),
 ('w-xpub','w','p','Xpub','xpub','{"xpub":"synthetic","script_types":["p2wpkh"]}','2025'),
 ('w-xpub-empty','w','p','Xpub empty','xpub','{"xpub":"synthetic","script_types":[]}','2025'),
 ('w-addresses','w','p','Addresses','address','{"addresses":["bcrt1q"]}','2025'),
 ('w-pay','w','p','Pay','descriptor','{"sync_source":"btcpay","descriptor":"wpkh(x)","backend":"btcpay1"}','2025'),
 ('w-file','w','p','File','descriptor','{"source_file":"a.csv","source_format":"csv","descriptor":"wpkh(x)"}','2025'),
 ('w-sp','w','p','SP','silent-payment','{"sp_descriptor":"sp(x)"}','2025'),
 ('w-bare','w','p','Bare','address','{"backend":"bare"}','2025'),
 ('w-missing','w','p','Missing','address','{"backend":"ghost"}','2025'),
 ('w-nonstring','w','p','Nonstring','address','{"backend":5,"descriptor":"x"}','2025'),
 ('w-empty','w','p','Empty','address','','2025'),
 ('w-list','w','p','List','descriptor','[1]','2025'),
 ('w-foreign','w','foreign','Foreign2','descriptor','{"backend":"unused","descriptor":"x"}','2025');
"#;

/// Backends created through the daemon, as the Python test did, with their
/// clock columns pinned so the goldens do not depend on the run's time.
fn create_backends(desk: &Desk, backends: &[&str]) {
    let requests: Vec<(&str, &str)> = backends
        .iter()
        .map(|args| ("ui.backends.create", *args))
        .collect();
    desk.setup(&requests);
    desk.change(
        "UPDATE backends SET created_at='2025-01-01T00:00:00Z', updated_at='2025-01-01T00:00:00Z';",
    );
}

fn set_default_backend(desk: &Desk, name: &str) {
    desk.setup(&[(
        "ui.backends.set_default",
        &format!("{{\"name\": \"{name}\"}}"),
    )]);
}

fn configure_backends(desk: &Desk) {
    create_backends(desk, BACKENDS);
    desk.change(BACKEND_WALLETS);
}

const BACKENDS_READ_ARGS: &[Option<&str>] = &[
    None,
    Some("null"),
    Some("[]"),
    Some("\"x\""),
    Some("0"),
    Some("{}"),
    Some("{\"unexpected\": true}"),
];

#[test]
fn backends_list_without_backends_or_profile() {
    let mut desk = Desk::new("backends_list_without_backends_or_profile");
    let responses = desk.session("ui.backends.list", BACKENDS_READ_ARGS, false);
    assert!(responses.iter().all(|r| kind_of(r) == "ui.backends.list"));
    desk.change("DELETE FROM settings WHERE key='context_profile';");
    desk.session("ui.backends.list", &[Some("{}")], false);
    desk.change("DELETE FROM settings WHERE key='context_workspace';");
    desk.session("ui.backends.list", &[Some("{}")], false);
    desk.check();
}

#[test]
fn backends_list_redacts_and_scopes_to_active_profile_wallets() {
    let mut desk = Desk::new("backends_list_redacts_and_scopes_to_active_profile_wallets");
    configure_backends(&desk);
    let responses = desk.session("ui.backends.list", BACKENDS_READ_ARGS, false);
    let names: Vec<&str> = list(&responses[0], &["data", "backends"])
        .iter()
        .map(|b| path(b, &["name"]).and_then(text).unwrap())
        .collect();
    assert!(
        names.contains(&"node") && !names.contains(&"unused"),
        "{names:?}"
    );
    for name in ["electrum1", "core", "unused"] {
        set_default_backend(&desk, name);
        desk.session("ui.backends.list", &[Some("{}")], false);
    }
    desk.change("UPDATE settings SET value='foreign' WHERE key='context_profile';");
    desk.session("ui.backends.list", &[Some("{}")], false);
    // No foreign wallet uses `core`, so the summary must not name it.
    set_default_backend(&desk, "core");
    let responses = desk.session("ui.backends.list", &[Some("{}")], false);
    assert_eq!(
        path(&responses[0], &["data", "summary", "default_backend"]),
        Some(&Value::Null)
    );
    desk.change("UPDATE workspaces SET label='' WHERE id='w';UPDATE profiles SET label='' WHERE id='foreign';");
    desk.session("ui.backends.list", &[Some("{}")], false);
    desk.check();
}

// --- ui.wallets.utxos (tests/test_rust_desktop_utxos.py) ---

const UTXO_BACKENDS: &[&str] = &[
    r#"{"name": "electrum1", "kind": "electrum", "url": "ssl://electrum.example:50002"}"#,
    r#"{"name": "node", "kind": "lnd", "url": "https://lnd.example:8080", "token": "abcd"}"#,
    r#"{"name": "unused", "kind": "esplora", "url": "https://unused.example/api"}"#,
    r#"{"name": "spcore", "kind": "bitcoinrpc", "url": "http://127.0.0.1:18443", "config": {"username": "synthetic-user", "password": "synthetic-password", "silent_payments": "yes"}}"#,
    r#"{"name": "regnet", "kind": "esplora", "url": "https://regnet.example/api", "chain": "bitcoin", "network": "regtest"}"#,
];

const UTXO_WALLETS: &str = r#"
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('u-desc','w','p','Desc','descriptor',
  '{"backend":"electrum1","descriptor":"wpkh(synthetic-private-descriptor)","chain":"bitcoin","network":"regtest","last_synced_at":"2025-03-02"}','2025'),
 ('u-label','w','p','u-desc','descriptor','{"backend":"electrum1","descriptor":"x"}','2025'),
 ('u-regnet','w','p','Regnet','descriptor','{"backend":"regnet","descriptor":"x","network":"main"}','2025'),
 ('u-addr','w','p','Addr','address','{"addresses":["bcrt1qsynthetic"]}','2025'),
 ('u-noaddr','w','p','No addr','address','{"backend":"electrum1"}','2025'),
 ('u-lnd','w','p','Lnd','descriptor','{"backend":"node","descriptor":"x"}','2025'),
 ('u-missing','w','p','Missing','descriptor','{"backend":"ghost","descriptor":"x"}','2025'),
 ('u-sp-esplora','w','p','SP esplora','silent-payment','{"sp_descriptor":"sp(x)","backend":"unused"}','2025'),
 ('u-sp-core','w','p','SP core','silent-payment','{"sp_descriptor":"sp(x)","backend":"spcore"}','2025'),
 ('u-sp-lnd','w','p','SP lnd','silent-payment','{"sp_descriptor":"sp(x)","backend":"node"}','2025'),
 ('u-liquid-bad','w','p','Liquid bad','descriptor','{"backend":"electrum1","chain":"liquid","descriptor":"wpkh(not-a-key)"}','2025'),
 ('u-liquid-nodesc','w','p','Liquid none','address','{"backend":"electrum1","chain":" Liquid ","addresses":["x"]}','2025'),
 ('u-wasabi','w','p','Wasabi','descriptor','{"source_format":"wasabi_bundle","source_file":"w.json","network":"regtest"}','2025'),
 ('u-pay','w','p','Pay','descriptor','{"sync_source":"btcpay","descriptor":"x","backend":"electrum1"}','2025'),
 ('u-never','w','p','Never','descriptor','{"backend":"electrum1","descriptor":"x","chain":"btc","network":"test"}','2025'),
 ('u-bad-config','w','p','Bad config','descriptor','not json','2025'),
 ('u-many','w','p','Many','descriptor','{"backend":"electrum1","descriptor":"x"}','2025'),
 ('f-desc','w','foreign','Foreign desc','descriptor','{"backend":"electrum1","descriptor":"x"}','2025');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,
  occurred_at,confirmed_at,created_at,kind,kind_override,fiat_rate,fiat_value,review_status,note,description,counterparty,fiat_currency)
VALUES ('u-tx','w','p','u-desc','u-tx','AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA','BTC','inbound',1,0,'2025-02-01T00:00:00Z',NULL,'2025','deposit',
  NULL,NULL,NULL,'completed','','','',NULL);
INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,backend_name,backend_kind,chain,network,asset,amount,txid,vout,
  outpoint,confirmation_status,confirmations,block_height,block_time,address,script_pubkey,address_label,branch_label,
  branch_index,address_index,anonymity_score,spent_by,excluded_from_coinjoin,key_state,anon_history_json,first_seen_at,
  last_seen_at,spent_at,raw_json) VALUES
 ('o1','w','p','u-desc','electrum1','electrum','bitcoin','regtest','BTC',150000000123,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',1,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1','confirmed',3,
  100,'2025-02-01T00:00:00Z','bcrt1qaddress','0014ff','Savings','receive',0,4,7,'',1,'active','[1,2]','2025-02-01','2025-03-01',
  NULL,'{"samourai":{"account":"postmix"},"other":1}'),
 ('o2','w','p','u-desc','electrum1','electrum','bitcoin','regtest','BTC',1001,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',0,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:0','unconfirmed',0,
  NULL,NULL,NULL,'0014aa',NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,'not json','2025-02-02','2025-02-03',NULL,'[1]'),
 ('o3','w','p','u-desc','electrum1','electrum','bitcoin','regtest','BTC',5000,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',1,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:1','confirmed',1,
  99,NULL,'bcrt1qspent',NULL,NULL,NULL,NULL,NULL,NULL,'cc',NULL,NULL,'{}','2025','2025','2025-03-03','{}'),
 ('o4','w','p','u-desc','old','electrum','bitcoin','regtest','BTC',777,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',2,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:2','confirmed',1,
  1,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'[]','2025','2025',NULL,'{}'),
 ('o5','w','p','u-desc','electrum1','electrum','bitcoin','regtest','ABC',3,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',3,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:3','confirmed',1,
  0,NULL,'','',NULL,NULL,NULL,NULL,NULL,NULL,NULL,'','{"a":1}','2025','2025',NULL,'{"samourai":3}'),
 ('o6','w','p','u-wasabi','wasabi','wasabi_bundle','bitcoin','regtest','BTC',42000,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',7,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:7','confirmed',9,
  50,NULL,NULL,NULL,NULL,NULL,NULL,NULL,5,NULL,0,NULL,'[]','2025','2025-04-01',NULL,'{}'),
 ('o7','w','p','u-regnet','regnet','esplora','bitcoin','regtest','BTC',8,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',8,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:8','confirmed',1,
  5,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'[]','2025','2025',NULL,'{}'),
 ('o8','w','p','u-regnet','regnet','esplora','bitcoin','main','BTC',9,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',9,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:9','confirmed',1,
  5,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'[]','2025','2025',NULL,'{}');
INSERT INTO wallet_utxo_refreshes(wallet_id,workspace_id,profile_id,backend_name,backend_kind,chain,network,observed_count,
  active_count,last_seen_at) VALUES ('u-desc','w','p','electrum1','electrum','bitcoin','regtest',4,3,'2025-03-05');
WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 500)
INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,backend_name,backend_kind,chain,network,asset,amount,txid,vout,
  outpoint,confirmation_status,first_seen_at,last_seen_at)
SELECT 'm' || i,'w','p','u-many','electrum1','electrum','bitcoin','main','BTC',1000 + i,
  printf('%064d', i),i,printf('%064d', i) || ':' || i,'confirmed','2025','2025' FROM n;
"#;

const UTXO_WALLET_IDS: &[&str] = &[
    "u-desc",
    "u-label",
    "u-regnet",
    "u-addr",
    "u-noaddr",
    "u-lnd",
    "u-missing",
    "u-sp-esplora",
    "u-sp-core",
    "u-sp-lnd",
    "u-liquid-bad",
    "u-liquid-nodesc",
    "u-wasabi",
    "u-pay",
    "u-never",
    "u-bad-config",
    "u-many",
];

const UTXO_REFUSALS: &[Option<&str>] = &[
    None,
    Some("null"),
    Some("[]"),
    Some("\"x\""),
    Some("3"),
    Some("{}"),
    Some(r#"{"wallet": ""}"#),
    Some(r#"{"wallet": "  "}"#),
    Some(r#"{"wallet": 3}"#),
    Some(r#"{"wallet": ["u-desc"]}"#),
    Some(r#"{"connection": 3}"#),
    Some(r#"{"wallet": "ghost"}"#),
    Some(r#"{"wallet": "f-desc"}"#),
    Some(r#"{"wallet": "foreign desc"}"#),
];

fn configure_utxos(desk: &Desk) {
    create_backends(desk, UTXO_BACKENDS);
    desk.change(UTXO_WALLETS);
}

#[test]
fn utxos_refusals_and_missing_profile() {
    let mut desk = Desk::new("utxos_refusals_and_missing_profile");
    configure_utxos(&desk);
    let responses = desk.session("ui.wallets.utxos", UTXO_REFUSALS, false);
    assert!(responses[..responses.len() - 2]
        .iter()
        .all(|r| kind_of(r) == "error"));
    desk.change("DELETE FROM settings WHERE key='context_profile';");
    let responses = desk.session("ui.wallets.utxos", &UTXO_REFUSALS[..4], false);
    assert!(responses.iter().all(|r| kind_of(r) == "ui.wallets.utxos"));
    desk.check();
}

#[test]
fn utxos_every_support_branch_and_inventory_matches_wire_bytes() {
    let mut desk = Desk::new("utxos_every_support_branch_and_inventory_matches_wire_bytes");
    configure_utxos(&desk);
    let mut args: Vec<String> = UTXO_WALLET_IDS
        .iter()
        .map(|id| format!("{{\"wallet\": \"{id}\"}}"))
        .collect();
    args.extend([
        r#"{"wallet": " Desc "}"#.to_owned(),
        r#"{"wallet": "", "connection": "Wasabi"}"#.to_owned(),
        r#"{"wallet": null, "connection": " u-desc "}"#.to_owned(),
    ]);
    let args: Vec<Option<&str>> = args.iter().map(|a| Some(a.as_str())).collect();
    let responses = desk.session("ui.wallets.utxos", &args, false);
    let desc = responses
        .iter()
        .find(|r| path(r, &["data", "wallet", "id"]).and_then(text) == Some("u-desc"))
        .unwrap();
    let ids: Vec<&str> = list(desc, &["data", "utxos"])
        .iter()
        .map(|u| path(u, &["id"]).and_then(text).unwrap())
        .collect();
    assert_eq!(ids, ["o5", "o1", "o2"]);
    let many = responses
        .iter()
        .find(|r| path(r, &["data", "wallet", "id"]).and_then(text) == Some("u-many"))
        .unwrap();
    assert_eq!(list(many, &["data", "utxos"]).len(), 500);
    for name in ["electrum1", "unused"] {
        set_default_backend(&desk, name);
        desk.session(
            "ui.wallets.utxos",
            &[
                Some(r#"{"wallet": "u-addr"}"#),
                Some(r#"{"wallet": "u-desc"}"#),
            ],
            false,
        );
    }
    desk.check();
}

/// Python crashed on these references (an unhandled UnicodeEncodeError);
/// Rust refuses with the same code and message and no traceback.
#[test]
fn utxos_surrogate_reference_matches_python_crash_code_and_message() {
    let mut desk = Desk::new("utxos_surrogate_reference_matches_python_crash_code_and_message");
    configure_utxos(&desk);
    let responses = desk.session(
        "ui.wallets.utxos",
        &[
            Some(r#"{"wallet": "\ud800"}"#),
            Some(r#"{"wallet": "a\udc00\udc01b"}"#),
            Some(r#"{"wallet": " \udfff "}"#),
        ],
        false,
    );
    assert!(responses.iter().all(|r| kind_of(r) == "error"));
    desk.check();
}

// --- ui.backends.lightning.test (tests/test_rust_desktop_lightning_test.py) ---

const LIGHTNING: &str = "ui.backends.lightning.test";

fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn insert_backend(name: &str, kind: &str, url: &str, token: &str, config: &str) -> String {
    format!(
        "INSERT INTO backends(name,kind,url,token,timeout,config_json,created_at,updated_at) \
         VALUES({},{},{},{},2,{},'fixed','fixed');",
        sql_quote(name),
        sql_quote(kind),
        sql_quote(url),
        sql_quote(token),
        sql_quote(config)
    )
}

/// The reference fixtures' good cases, as the Python test picked them.
/// The fixtures hold lone surrogates, so they are read as Python reads them.
fn lightning_cases(kind: &str) -> Vec<Value> {
    let file = if kind == "coreln" {
        "python_cln.json"
    } else {
        "python_lightning.json"
    };
    let text = std::fs::read_to_string(
        support::repository()
            .join("crates/chain/fixtures")
            .join(file),
    )
    .unwrap();
    let Value::List(cases) = kassiber_pytext::loads(&text).unwrap() else {
        panic!("{file} is not a list")
    };
    let good: Vec<Value> = cases
        .into_iter()
        .filter(|c| {
            get(c, "expected").is_some()
                && !get(c, "failures").is_some_and(kassiber_envelope::truthy)
        })
        .collect();
    vec![
        good[0].clone(),
        good[good.len() / 2].clone(),
        good[good.len() - 1].clone(),
    ]
}

fn key_text(key: &kassiber_pyjson::Key) -> String {
    match key {
        kassiber_pyjson::Key::Str(text) => text.as_str().unwrap().to_owned(),
        other => panic!("not a string key: {other:?}"),
    }
}

/// `json.dumps(value)`.
fn dumps(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new()).unwrap()
}

fn items(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        other => panic!("not a list: {other:?}"),
    }
}

/// An owned loopback LND that answers the case's calls in order.
fn lnd_node(calls: &[Value]) -> kassiber_net::test_support::FakeProvider {
    kassiber_net::test_support::FakeProvider::replies(
        calls
            .iter()
            .map(|call| (200, dumps(get(call, "response").unwrap())))
            .collect(),
    )
    .unwrap()
}

/// The node's request lines, in order.
fn node_hits(node: &kassiber_net::test_support::FakeProvider) -> Vec<String> {
    node.requests()
        .iter()
        .map(|r| r.lines().next().unwrap_or_default().to_owned())
        .collect()
}

fn lightning_desk(golden: &str) -> Desk {
    let mut desk = Desk::new(golden);
    // Loopback stays allowed by the test observer; the product switch
    // would refuse every node request.
    desk.env.remove("KASSIBER_NO_EGRESS");
    desk
}

#[test]
fn lightning_refusals_and_non_lightning_backends_make_no_node_traffic() {
    let mut desk =
        lightning_desk("lightning_refusals_and_non_lightning_backends_make_no_node_traffic");
    let case = &lightning_cases("lnd")[0];
    let node = lnd_node(items(get(case, "calls").unwrap()));
    desk.replacements
        .push((node.base().to_owned(), "$NODE".to_owned()));
    desk.change(
        &(insert_backend("chain", "esplora", "https://chain.example", "", "{}")
            + &insert_backend("nwcnode", "nwc", "nostr+walletconnect://x", "", "{}")
            + &insert_backend("node", "lnd", node.base(), "aabb", "{}")),
    );
    let responses = desk.session(
        LIGHTNING,
        &[
            None,
            Some("null"),
            Some("[]"),
            Some("\"x\""),
            Some("{}"),
            Some(r#"{"backend": ""}"#),
            Some(r#"{"backend": "  "}"#),
            Some(r#"{"backend": 3}"#),
            Some(r#"{"backend": "missing"}"#),
            Some(r#"{"backend": "chain"}"#),
            Some(r#"{"backend": " NWCNODE "}"#),
        ],
        false,
    );
    assert_eq!(node_hits(&node), Vec::<String>::new());
    let logs = list(&responses[9], &["data", "logs"]);
    assert_eq!(
        logs.last().and_then(text),
        Some("Backend kind 'esplora' is not a Lightning node backend.")
    );
    assert_eq!(
        path(&responses[10], &["data", "error", "code"]).and_then(text),
        Some("lightning_adapter_unavailable")
    );
    desk.check();
}

#[test]
fn lightning_lnd_node_fields_follow_the_reference_coercions() {
    let mut desk = lightning_desk("lightning_lnd_node_fields_follow_the_reference_coercions");
    let base = &lightning_cases("lnd")[0];
    let infos = [
        r#"{"alias": "Synthetic Node", "num_peers": "7", "block_height": 812345, "chains": [{"chain": "bitcoin", "network": " REGTEST "}]}"#,
        r#"{"alias": 5, "num_peers": "x", "block_height": 0, "testnet": true}"#,
        r#"{"alias": "", "num_peers": 2.9, "block_height": "17", "chains": [{"network": ""}, {"network": "sig"}]}"#,
        r#"{"testnet": "Signet", "chains": "bitcoin"}"#,
        r#"{"testnet": false}"#,
    ];
    let mut last = Vec::new();
    for info in infos {
        let mut calls = items(get(base, "calls").unwrap()).to_vec();
        let Value::Dict(info) = kassiber_pytext::loads(info).unwrap() else {
            panic!("info")
        };
        let mut response = get(&calls[0], "response").unwrap().clone();
        for (key, value) in info {
            kassiber_envelope::set(&mut response, &key_text(&key), value);
        }
        kassiber_envelope::set(&mut calls[0], "response", response);
        let node = lnd_node(&calls);
        desk.replacements.truncate(1);
        desk.replacements
            .push((node.base().to_owned(), "$NODE".to_owned()));
        desk.change(
            &("DELETE FROM backends;".to_owned()
                + &insert_backend("node", "lnd", node.base(), "aabb", "{}")),
        );
        last = desk.session(LIGHTNING, &[Some(r#"{"backend": "node"}"#)], false);
        assert_eq!(path(&last[0], &["data", "ok"]), Some(&Value::Bool(true)));
        for hit in node_hits(&node) {
            desk.golden.line(&format!("node {hit}"));
        }
    }
    assert_eq!(
        path(&last[0], &["data", "network"]).and_then(text),
        Some("mainnet")
    );
    desk.check();
}

/// A fake `lightning-cli`: answers each method from `<rpc-file>/<method>.json`
/// and logs its arguments to `<rpc-file>/hits`.
fn fake_cln(desk: &Desk, data: &Value) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = desk.base_path().join("cln");
    std::fs::create_dir_all(&dir).unwrap();
    let Value::Dict(methods) = data else {
        panic!("data")
    };
    for (method, reply) in methods {
        let method = key_text(method);
        std::fs::write(dir.join(format!("{method}.json")), dumps(reply)).unwrap();
    }
    let cli = desk.base_path().join("fake-lightning-cli");
    std::fs::write(
        &cli,
        "#!/bin/sh\nfor a; do case $a in --rpc-file=*) dir=${a#--rpc-file=};; esac; method=$a; done\n\
         printf '%s\\n' \"$*\" >> \"$dir/hits\"\nexec cat \"$dir/$method.json\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    (cli, dir)
}

#[test]
fn lightning_cln_snapshot_matches_bytes_and_commands() {
    let mut desk = lightning_desk("lightning_cln_snapshot_matches_bytes_and_commands");
    let case = &lightning_cases("coreln")[0];
    let (cli, dir) = fake_cln(&desk, get(case, "data").unwrap());
    desk.env.insert(
        "KASSIBER_RUST_TEST_CLN_FIXTURE".to_owned(),
        cli.to_string_lossy().into_owned(),
    );
    let config = serde_json::json!({
        "rpc_file": dir.to_string_lossy(),
        "lightning_cli": cli.to_string_lossy(),
    });
    desk.change(&insert_backend(
        "cln",
        "coreln",
        "cln://local",
        "",
        &config.to_string(),
    ));
    let responses = desk.session(LIGHTNING, &[Some(r#"{"backend": "cln"}"#)], false);
    assert_eq!(
        path(&responses[0], &["data", "ok"]),
        Some(&Value::Bool(true))
    );
    let hits = std::fs::read_to_string(dir.join("hits")).unwrap();
    for hit in hits.lines() {
        let hit = desk.normalize(hit);
        desk.golden.line(&format!("cli {hit}"));
    }
    desk.check();
}

#[test]
fn lightning_failures_report_ok_false_with_the_reference_message() {
    let mut desk = lightning_desk("lightning_failures_report_ok_false_with_the_reference_message");
    let case = &lightning_cases("lnd")[0];
    let node = lnd_node(items(get(case, "calls").unwrap()));
    desk.replacements
        .push((node.base().to_owned(), "$NODE".to_owned()));
    desk.change(
        &(insert_backend("nomacaroon", "lnd", node.base(), "", "{}")
            + &insert_backend("badmacaroon", "lnd", node.base(), "not-hex", "{}")
            + &insert_backend("nourl", "lnd", "", "aabb", "{}")),
    );
    let responses = desk.session(
        LIGHTNING,
        &[
            Some(r#"{"backend": "nomacaroon"}"#),
            Some(r#"{"backend": "badmacaroon"}"#),
            Some(r#"{"backend": "nourl"}"#),
        ],
        false,
    );
    assert_eq!(node_hits(&node), Vec::<String>::new());
    assert!(responses
        .iter()
        .all(|r| path(r, &["data", "ok"]) == Some(&Value::Bool(false))));
    desk.check();
}
