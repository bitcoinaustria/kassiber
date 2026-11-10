//! The review workflow's daemon kinds and CLI leaves against frozen goldens
//! (see `support`). These replaced the Python-compared cases in
//! `tests/test_rust_review_terms.py`, `tests/test_rust_review_workflow.py`
//! and `tests/test_rust_component_remediation.py`. Generated identities and
//! the run's own clocks are masked as those tests' maskers did.

#![cfg(feature = "test-harness")]

mod support;

use kassiber_envelope::{get, set};
use kassiber_pyjson::Value;
use support::{kind_of, list, path, text, Desk, MaskSpec};

const NOW: &str = "2026-01-01T00:00:00Z";

/// `ReviewParity.seed` (tests/test_rust_review_workflow.py).
fn review_seed() -> String {
    let mut sql = format!(
        "INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main','{NOW}');\
         INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('profile','ws','Book','{NOW}');\
         INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) \
         VALUES('treasury','ws','profile','treasury','Treasury','asset','{NOW}');"
    );
    for (wallet, label) in [("source-wallet", "Source"), ("sink-wallet", "Sink")] {
        sql += &format!(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) \
             VALUES('{wallet}','ws','profile','{label}','custom','{{}}','{NOW}');"
        );
    }
    for (tx, wallet, direction, amount, rate, occurred) in [
        ("out", "source-wallet", "outbound", 100_000, "NULL", NOW),
        ("in", "sink-wallet", "inbound", 99_000, "NULL", NOW),
        (
            "buy",
            "sink-wallet",
            "inbound",
            5_000_000,
            "NULL",
            "2024-02-01T00:00:00Z",
        ),
        (
            "sell",
            "sink-wallet",
            "outbound",
            1_000_000,
            "50000",
            "2024-03-01T00:00:00Z",
        ),
        (
            "gift",
            "source-wallet",
            "inbound",
            7_000_000,
            "40000",
            "2023-06-01T00:00:00Z",
        ),
    ] {
        sql += &format!(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,\
             occurred_at,direction,asset,amount,fee,fiat_rate,created_at) \
             VALUES('{tx}','ws','profile','{wallet}','fingerprint-{tx}','external-{tx}','{occurred}',\
             '{direction}','BTC',{amount},0,{rate},'{occurred}');"
        );
    }
    sql + "INSERT INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','profile');"
}

const COARSE: &str = "UPDATE profiles SET require_coarse_review=1;\
     UPDATE transactions SET fiat_rate=40000.0,fiat_rate_exact='40000',\
     pricing_quality='coarse_fallback',pricing_source_kind='fmv_provider',\
     pricing_granularity='daily' WHERE id IN ('buy','gift');";

fn review_desk(golden: &str, sql: &str) -> Desk {
    let desk = Desk::with_seed(golden, None);
    desk.change(sql);
    desk
}

/// Mask what this test's runs generate from here on: identities the book
/// holds now stay as stored (`SyncMasker`, every UUID version, new hashes).
fn mask_generated(desk: &mut Desk, since: &str) {
    desk.mask = Some(MaskSpec {
        any_version: true,
        hashes: true,
        known: support::known_identities(&desk.tables()),
        since: since.to_owned(),
    });
}

/// `Masker` of tests/test_rust_review_workflow.py: uuid4s and clocks only.
fn mask_uuid4(desk: &mut Desk, since: &str) {
    desk.mask = Some(MaskSpec {
        since: since.to_owned(),
        ..MaskSpec::default()
    });
}

fn input_version(desk: &Desk) -> i64 {
    desk.connection()
        .query_row(
            "SELECT COALESCE(journal_input_version, 0) FROM profiles",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn plan(operations: &[String], version: i64) -> String {
    format!(
        r#"{{"operations": [{}], "expected_input_version": {version}}}"#,
        operations.join(", ")
    )
}

fn pair(extra: &str) -> String {
    format!(
        r#"{{"type": "pair", "out_transaction_id": "out", "in_transaction_id": "in", "reason": "Same owner"{extra}}}"#
    )
}

fn accept(extra: &str) -> String {
    format!(r#"{{"type": "accept_coarse_prices", "reason": "Daily averages reviewed"{extra}}}"#)
}

fn dumps(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new()).unwrap()
}

/// The artifact `ui.review.plan` returns for `operations` (read-only).
fn planned(desk: &Desk, operations: &[String]) -> Value {
    let plan = desk.ask("ui.review.plan", &plan(operations, input_version(desk)));
    assert_eq!(kind_of(&plan), "ui.review.plan", "{plan:?}");
    get(&plan, "data").unwrap().clone()
}

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

#[test]
fn review_coarse_cases_offer_acceptance_and_plans_name_the_cases() {
    let mut desk = review_desk(
        "review_coarse_cases_offer_acceptance_and_plans_name_the_cases",
        &(review_seed() + COARSE),
    );
    let cases = desk.session("ui.review.cases", &[Some("{}")], false);
    assert_eq!(
        path(&cases[0], &["data", "fiat_currency"]).and_then(text),
        Some("USD")
    );
    let version = input_version(&desk);
    let plans: Vec<String> = [
        vec![accept("")],
        vec![accept(r#", "transaction_ids": ["buy"]"#)],
        vec![accept(r#", "transaction_ids": ["buy", "sell"]"#)],
        vec![
            r#"{"type": "exclude", "transaction_id": "buy", "reason": "x"}"#.to_owned(),
            accept(""),
        ],
        vec![accept(""), accept("")],
        vec![
            r#"{"type": "price_override", "transaction_id": "buy", "reason": "x", "fiat_rate": "41000"}"#
                .to_owned(),
            accept(r#", "transaction_ids": ["buy"]"#),
        ],
    ]
    .iter()
    .map(|operations| plan(operations, version))
    .collect();
    let args: Vec<Option<&str>> = plans.iter().map(|p| Some(p.as_str())).collect();
    let responses = desk.session("ui.review.plan", &args, false);
    let operation = &list(&responses[0], &["data", "operations"])[0];
    assert!(get(operation, "transaction_ids").is_some());
    desk.check();
}

#[test]
fn review_count_journal_runs_recount_and_badges_match_python() {
    let since = support::started();
    let mut desk = review_desk(
        "review_count_journal_runs_recount_and_badges_match_python",
        &review_seed(),
    );
    desk.setup_cli(&["transfers", "pair", "--tx-out", "out", "--tx-in", "in"]);
    mask_generated(&mut desk, &since);
    desk.session("ui.review.badges", &[Some("{}"), None], false);
    desk.cli(
        &[argv(&["journals", "process"]), argv(&["review", "cases"])],
        true,
    );
    desk.session("ui.review.badges", &[Some("{}")], false);
    desk.cli(&[argv(&["journals", "process", "--auto-pair"])], true);
    desk.session("ui.review.badges", &[Some("{}")], false);
    desk.check();
}

#[test]
fn review_term_apply_pair_apply_recounts_candidates_like_python() {
    let since = support::started();
    let mut desk = review_desk(
        "review_term_apply_pair_apply_recounts_candidates_like_python",
        &review_seed(),
    );
    let artifact = planned(&desk, &[pair("")]);
    mask_generated(&mut desk, &since);
    let apply = format!(
        r#"{{"artifact": {}, "idempotency_key": "pair"}}"#,
        dumps(&artifact)
    );
    desk.session("ui.review.apply", &[Some(&apply), Some(&apply)], true);
    desk.session(
        "ui.review.receipt",
        &[Some(r#"{"idempotency_key": "pair"}"#)],
        false,
    );
    desk.session("ui.review.cases", &[Some("{}")], false);
    desk.session("ui.review.badges", &[Some("{}")], false);
    let count: Option<i64> = desk
        .connection()
        .query_row("SELECT swap_candidate_count FROM profiles", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(count.is_some());
    desk.check();
}

/// `AustrianTaxDeltaParity.seed`.
fn austrian_seed() -> String {
    let mut sql = format!(
        "INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main','{NOW}');\
         INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) \
         VALUES('profile','ws','Book','EUR','at','{NOW}');\
         INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) \
         VALUES('treasury','ws','profile','treasury','Treasury','asset','{NOW}');\
         INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) \
         VALUES('wallet','ws','profile','Main','custom','{{}}','{NOW}');"
    );
    for (tx, direction, amount, rate, occurred) in [
        (
            "alt-buy",
            "inbound",
            50_000_000_000_i64,
            "8000",
            "2020-01-01T10:00:00Z",
        ),
        (
            "alt-sell",
            "outbound",
            50_000_000_000,
            "15000",
            "2020-12-01T10:00:00Z",
        ),
        (
            "buy",
            "inbound",
            100_000_000_000,
            "30000",
            "2021-06-01T10:00:00Z",
        ),
        (
            "early",
            "outbound",
            20_000_000_000,
            "40000",
            "2022-02-15T10:00:00Z",
        ),
        (
            "loss",
            "outbound",
            30_000_000_000,
            "25000",
            "2022-06-01T10:00:00Z",
        ),
        (
            "unpriced",
            "inbound",
            5_000_000_000,
            "NULL",
            "2023-01-01T10:00:00Z",
        ),
        (
            "gain",
            "outbound",
            10_000_000_000,
            "50000",
            "2023-05-01T10:00:00Z",
        ),
        (
            "new-year",
            "outbound",
            1_000_000_000,
            "60000",
            "2024-12-31T23:30:00Z",
        ),
    ] {
        sql += &format!(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,\
             occurred_at,direction,asset,amount,fee,fiat_rate,fiat_currency,created_at) \
             VALUES('{tx}','ws','profile','wallet','fingerprint-{tx}','external-{tx}','{occurred}',\
             '{direction}','BTC',{amount},0,{rate},'EUR','{occurred}');"
        );
    }
    sql + "INSERT INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','profile');"
}

/// #711: a price entered for a row imported in another currency is the
/// book's.
#[test]
fn review_austrian_reviewed_prices_are_stored_in_the_book_currency() {
    let since = support::started();
    let mut desk = review_desk(
        "review_austrian_reviewed_prices_are_stored_in_the_book_currency",
        &austrian_seed(),
    );
    desk.change("UPDATE transactions SET fiat_currency='USD' WHERE id='unpriced'");
    desk.setup_cli(&["journals", "process"]);
    mask_generated(&mut desk, &since);
    desk.session(
        "ui.journals.quarantine.resolve",
        &[Some(
            r#"{"transaction": "unpriced", "action": "price_override", "fiat_rate": "20000", "reason": "Receipt"}"#,
        )],
        true,
    );
    let currency: String = desk
        .connection()
        .query_row(
            "SELECT fiat_currency FROM transactions WHERE id='unpriced'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(currency, "EUR");
    desk.check();
}

// --- a pair authored before observed-fee separation (ReviewLegacyPairFeeParity) ---

/// Both runtimes authored the repair at one clock: the revision's component
/// id hashes its `created_at`.
const REPAIR_CLOCK: &str = "2026-11-02T00:00:00Z";

fn legacy_desk(golden: &str) -> Desk {
    let mut desk = Desk::from_book(golden, "legacy-pair-fee-c0a015de");
    desk.env.insert(
        "KASSIBER_RUST_TEST_JOURNAL_TIME".to_owned(),
        REPAIR_CLOCK.to_owned(),
    );
    desk
}

fn repair(desk: &Desk) -> String {
    let cases = desk.ask("ui.review.cases", r#"{"limit": 100}"#);
    let case = list(&cases, &["data", "cases"])
        .iter()
        .find(|case| get(case, "repair").is_some())
        .expect("a repair case");
    let pair_id = path(case, &["repair", "pair_id"]).and_then(text).unwrap();
    let fingerprint = path(case, &["pair", "pair_fingerprint"])
        .and_then(text)
        .unwrap();
    format!(
        r#"{{"type": "repair_legacy_pair", "pair_id": "{pair_id}", "expected_fingerprint": "{fingerprint}", "reason": "Reviewed complete debit"}}"#
    )
}

#[test]
fn review_legacy_repair_apply_matches_python() {
    let since = support::started();
    let mut desk = legacy_desk("review_legacy_repair_apply_matches_python");
    let artifact = planned(&desk, &[repair(&desk)]);
    mask_uuid4(&mut desk, &since);
    let apply = format!(
        r#"{{"artifact": {}, "idempotency_key": "repair"}}"#,
        dumps(&artifact)
    );
    desk.session("ui.review.apply", &[Some(&apply)], true);
    desk.session("ui.review.cases", &[Some("{}")], false);
    desk.check();
}

/// Each run draws fresh uuid4s: the rows are identical once only those are
/// masked, and the revision's ids and evidence commitments are identical as
/// stored.
#[test]
fn review_legacy_repair_apply_stores_the_same_rows_on_every_run() {
    let desks = [
        legacy_desk("review_legacy_repair_apply_stores_the_same_rows_on_every_run"),
        legacy_desk("unused"),
    ];
    let artifact = planned(&desks[0], &[repair(&desks[0])]);
    let apply = format!(
        r#"{{"artifact": {}, "idempotency_key": "repair"}}"#,
        dumps(&artifact)
    );
    let mut runs = Vec::new();
    for mut desk in desks {
        let before = desk.tables();
        let mut masker = support::Masker::new(MaskSpec {
            since: "9999-12-31T23:59:59".to_owned(),
            ..MaskSpec::default()
        });
        let wire = format!(
            "{}\n{{\"request_id\": \"stop\", \"kind\": \"daemon.shutdown\"}}\n",
            format_args!(
                r#"{{"request_id": "apply", "kind": "ui.review.apply", "args": {apply}}}"#
            )
        );
        let output = desk.run(&["daemon"], wire.as_bytes());
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<String> = stdout
            .lines()
            .skip(1)
            .map(|line| masker.text(&desk.replace_run_text(line)))
            .collect();
        let tables = desk.tables();
        let masked = masker.tables(&tables);
        if desk.golden.text().is_empty() {
            for line in &lines {
                desk.golden.line(&format!("< {line}"));
            }
            for line in support::table_changes(&masker.tables(&before), &masked) {
                desk.golden.line(&line);
            }
        }
        runs.push((lines, masked, tables, desk));
    }
    let (second, first) = (runs.pop().unwrap(), runs.pop().unwrap());
    assert_eq!(first.0, second.0);
    assert_eq!(first.1, second.1);
    for table in [
        "custody_components",
        "custody_component_evidence_commitments",
        "custody_component_economic_terms",
    ] {
        let revised = |tables: &std::collections::BTreeMap<String, Vec<String>>| -> Vec<String> {
            tables[table]
                .iter()
                .filter(|row| row.contains(REPAIR_CLOCK))
                .cloned()
                .collect()
        };
        assert!(!revised(&first.2).is_empty(), "{table}");
        assert_eq!(revised(&first.2), revised(&second.2), "{table}");
    }
    first.3.check();
}

// --- the review CLI leaves (ReviewCliParity) ---

fn price(tx: &str) -> String {
    format!(
        r#"{{"type": "price_override", "transaction_id": "{tx}", "reason": "Verified exchange rate", "fiat_rate": "45000"}}"#
    )
}

fn write_input(desk: &Desk, name: &str, payload: &[u8]) -> String {
    let path = desk.base_path().join("inputs").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, payload).unwrap();
    path.to_string_lossy().into_owned()
}

fn envelope(kind: Value, data: Option<&Value>) -> Value {
    let mut value = kassiber_envelope::dict([("kind", kind)]);
    if let Some(data) = data {
        set(&mut value, "data", data.clone());
    }
    value
}

#[test]
fn review_cli_apply_artifacts_envelopes_and_retries_match_python() {
    let since = support::started();
    let mut desk = review_desk(
        "review_cli_apply_artifacts_envelopes_and_retries_match_python",
        &review_seed(),
    );
    let artifact = planned(
        &desk,
        &[
            price("buy"),
            r#"{"type": "exclude", "transaction_id": "sell", "reason": "Duplicate"}"#.to_owned(),
        ],
    );
    let other = planned(
        &desk,
        &[r#"{"type": "exclude", "transaction_id": "buy", "reason": "Not mine"}"#.to_owned()],
    );
    mask_uuid4(&mut desk, &since);
    let file =
        |desk: &Desk, name: &str, value: &Value| write_input(desk, name, dumps(value).as_bytes());
    let raw = file(&desk, "artifact.json", &artifact);
    let mut tampered = artifact.clone();
    set(&mut tampered, "digest", Value::Str("0".repeat(64).into()));
    let s = |t: &str| Value::Str(t.into());
    let commands = [
        ("list.json", Value::List(vec![artifact.clone()]), "k"),
        (
            "unhashable.json",
            envelope(Value::List(vec![s("review.plan")]), Some(&artifact)),
            "k",
        ),
        (
            "object-kind.json",
            envelope(
                kassiber_envelope::dict([("plan", Value::from(1))]),
                Some(&artifact),
            ),
            "k",
        ),
        ("empty-envelope.json", envelope(s("review.plan"), None), "k"),
        ("tampered.json", tampered, "k"),
    ];
    let mut argvs: Vec<Vec<String>> = commands
        .iter()
        .map(|(name, value, key)| {
            argv(&[
                "review",
                "apply",
                "--artifact-file",
                &file(&desk, name, value),
                "--idempotency-key",
                key,
            ])
        })
        .collect();
    argvs.push(argv(&[
        "review",
        "apply",
        "--artifact-file",
        &raw,
        "--idempotency-key",
        " ",
    ]));
    argvs.push(argv(&[
        "review",
        "apply",
        "--artifact-file",
        &raw,
        "--idempotency-key",
        "cli-key",
    ]));
    for (name, value, key) in [
        (
            "envelope.json",
            envelope(s("review.plan"), Some(&artifact)),
            "cli-key",
        ),
        (
            "desktop.json",
            envelope(s("ui.review.plan"), Some(&other)),
            "cli-key",
        ),
        ("other.json", other.clone(), "second"),
    ] {
        argvs.push(argv(&[
            "review",
            "apply",
            "--artifact-file",
            &file(&desk, name, &value),
            "--idempotency-key",
            key,
        ]));
    }
    argvs.push(argv(&["review", "receipt", "--idempotency-key", "cli-key"]));
    argvs.push(argv(&["review", "cases"]));
    desk.cli(&argvs, true);
    desk.check();
}

// --- custody component remediation (tests/test_rust_component_remediation.py) ---

const SHARED: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const FRESH: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn component_seed() -> String {
    let mut sql = format!(
        "INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main','{NOW}');\
         INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('profile','ws','Book','{NOW}');\
         INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) \
         VALUES('treasury','ws','profile','treasury','Treasury','asset','{NOW}');"
    );
    for (wallet, label) in [("source-wallet", "Source"), ("sink-wallet", "Sink")] {
        sql += &format!(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) \
             VALUES('{wallet}','ws','profile','{label}','custom','{{}}','{NOW}');"
        );
    }
    for (tx, wallet, external, direction, amount) in [
        ("out", "source-wallet", SHARED, "outbound", 100_000),
        ("in", "sink-wallet", SHARED, "inbound", 99_000),
        ("out2", "source-wallet", FRESH, "outbound", 100_000),
        ("in2", "sink-wallet", FRESH, "inbound", 99_000),
    ] {
        sql += &format!(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,\
             occurred_at,direction,asset,amount,fee,created_at) VALUES('{tx}','ws','profile','{wallet}',\
             'fingerprint-{tx}','{external}','{NOW}','{direction}','BTC',{amount},0,'{NOW}');"
        );
    }
    sql + "INSERT OR REPLACE INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','profile');"
}

/// A list or object action is validation, after the scope and before the
/// apply's input version (Python fix b2c0d9447).
#[test]
fn review_component_non_text_actions_are_refused_like_python() {
    let mut desk = review_desk(
        "review_component_non_text_actions_are_refused_like_python",
        &component_seed(),
    );
    let spec = format!(
        r#"{{"component_type": "manual_bridge", "evidence_kind": "manual_claim", "evidence_grade": "reviewed", "notes": "x", "legs": [{{"role": "source", "transaction": "{SHARED}:out", "amount_msat": 100000}}, {{"role": "destination", "transaction": "{SHARED}:Sink", "amount_msat": 99000}}, {{"role": "fee", "transaction": "{SHARED}:out", "amount_msat": 1000}}]}}"#
    );
    let first = format!(r#"{{"action": ["create"], "components": [{spec}]}}"#);
    let refused = desk.session(
        "ui.transfers.components.plan",
        &[
            Some(&first),
            Some(r#"{"action": {"a": 1}}"#),
            Some(r#"{"action": null}"#),
            Some(r#"{"action": 7}"#),
            Some(r#"{"action": "explode"}"#),
            Some(r#"{"action": [], "profile": "missing"}"#),
        ],
        false,
    );
    assert_eq!(
        dumps(get(&refused[0], "error").unwrap()),
        r#"{"code": "validation", "message": "Custody component review action is unsupported", "hint": null, "details": null, "retryable": false, "debug": null}"#
    );
    assert_eq!(
        dumps(path(&refused[4], &["error", "details"]).unwrap()),
        r#"{"action": "explode"}"#
    );
    let applied = desk.session(
        "ui.transfers.components.apply",
        &[
            Some(r#"{"action": [], "expected_input_version": "stale"}"#),
            Some(r#"{"action": null, "expected_input_version": -1}"#),
            Some(r#"{"action": "explode", "expected_input_version": "stale"}"#),
        ],
        false,
    );
    assert_eq!(
        path(&applied[0], &["error", "message"]).and_then(text),
        Some("Custody component review action is unsupported")
    );
    assert!(path(&applied[2], &["error", "message"])
        .and_then(text)
        .unwrap()
        .contains("input version"));
    desk.check();
}
