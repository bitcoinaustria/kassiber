//! Point-in-time wealth statement (Stichtag Vermögensübersicht) and watermarked
//! draft export tests.

use kassiber_envelope::{dict, get, py_str, s};
use kassiber_pyjson::Value;
use kassiber_reports::report_design;
use kassiber_store::{rusqlite::Connection, Book};

struct Fixed;
impl kassiber_setup::Sources for Fixed {
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-03-15T12:00:00Z".into())
    }
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(format!("00000000-0000-4000-8000-{n:012}"))
    }
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&Value::Null)
}

fn num(v: &Value) -> f64 {
    py_str(v).parse::<f64>().unwrap()
}

fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(items) => items,
        _ => &[],
    }
}

fn seed_austrian_book(conn: &Connection) {
    conn.execute_batch(
        r#"
        INSERT INTO workspaces(id, label, created_at)
        VALUES ('ws-1', 'Privat', '2020-01-01T00:00:00Z');

        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            gains_algorithm, cost_basis_pool_scope,
            last_processed_at, last_processed_tx_count,
            journal_input_version, last_processed_input_version,
            created_at
        ) VALUES (
            'prof-at', 'ws-1', 'Hauptbuch AT', 'EUR', 'at',
            'moving_average', 'global',
            '2026-02-01T10:00:00Z', 3,
            5, 5,
            '2020-01-01T00:00:00Z'
        );

        INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
        VALUES
            ('w-cold', 'ws-1', 'prof-at', 'Coldcard Tresor', 'descriptor', '{"chain":"bitcoin","network":"mainnet"}', '2020-01-01T00:00:00Z'),
            ('w-hot',  'ws-1', 'prof-at', 'Sparrow Alltag',  'descriptor', '{"chain":"bitcoin","network":"mainnet"}', '2024-01-01T00:00:00Z');

        -- Altbestand: 1.0 BTC bought 2020-06-15 for EUR 8,500
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at, direction, asset,
            amount, fee, fiat_value, excluded, created_at
        ) VALUES (
            'tx-alt', 'ws-1', 'prof-at', 'w-cold', 'fp-alt', '2020-06-15T10:00:00Z', 'inbound', 'BTC',
            100000000000, 0, 8500.0, 0, '2020-06-15T10:00:00Z'
        );
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, wallet_id, transaction_id, occurred_at,
            entry_type, asset, quantity, fiat_value, cost_basis,
            created_at
        ) VALUES (
            'je-alt', 'ws-1', 'prof-at', 'w-cold', 'tx-alt', '2020-06-15T10:00:00Z',
            'acquisition', 'BTC', 100000000000, 8500.0, 8500.0,
            '2026-02-01T10:00:00Z'
        );

        -- Neubestand: 0.5 BTC bought 2024-03-10 for EUR 32,000 (avg EUR 64,000/BTC)
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at, direction, asset,
            amount, fee, fiat_value, excluded, created_at
        ) VALUES (
            'tx-neu', 'ws-1', 'prof-at', 'w-hot', 'fp-neu', '2024-03-10T14:00:00Z', 'inbound', 'BTC',
            50000000000, 0, 32000.0, 0, '2024-03-10T14:00:00Z'
        );
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, wallet_id, transaction_id, occurred_at,
            entry_type, asset, quantity, fiat_value, cost_basis,
            created_at
        ) VALUES (
            'je-neu', 'ws-1', 'prof-at', 'w-hot', 'tx-neu', '2024-03-10T14:00:00Z',
            'acquisition', 'BTC', 50000000000, 32000.0, 32000.0,
            '2026-02-01T10:00:00Z'
        );

        -- Post-Stichtag acquisition: 0.2 BTC on 2026-01-15 (must be excluded for as_of=2025-12-31)
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at, direction, asset,
            amount, fee, fiat_value, excluded, created_at
        ) VALUES (
            'tx-post', 'ws-1', 'prof-at', 'w-hot', 'fp-post', '2026-01-15T12:00:00Z', 'inbound', 'BTC',
            20000000000, 0, 18000.0, 0, '2026-01-15T12:00:00Z'
        );
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, wallet_id, transaction_id, occurred_at,
            entry_type, asset, quantity, fiat_value, cost_basis,
            created_at
        ) VALUES (
            'je-post', 'ws-1', 'prof-at', 'w-hot', 'tx-post', '2026-01-15T12:00:00Z',
            'acquisition', 'BTC', 20000000000, 18000.0, 18000.0,
            '2026-02-01T10:00:00Z'
        );

        -- Year-end 2025 FX rate (Vienna midnight 2025-12-31 23:00 UTC): EUR 58,000/BTC
        -- Below Neubestand cost basis (EUR 64,000/BTC) -> Niederstwert triggers!
        INSERT INTO rates_cache(pair, timestamp, rate, rate_exact, source, fetched_at)
        VALUES ('BTC-EUR', '2025-12-31T22:55:00Z', 58000.0, '58000', 'kraken_csv', '2026-01-01T00:00:00Z');
        "#,
    )
    .unwrap();
}

#[test]
fn austrian_wealth_statement_splits_alt_neu_and_computes_lower_of_cost_or_market() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);

    let args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
    ]);
    let report =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &args, &Fixed).unwrap();
    assert_eq!(py_str(field(&report, "status")), "final");
    assert_eq!(field(&report, "isDraft"), &Value::Bool(false));
    assert_eq!(py_str(field(&report, "asOfDate")), "2025-12-31");
    assert_eq!(field(&report, "includeRegime"), &Value::Bool(true));
    assert_eq!(field(&report, "includeValuation"), &Value::Bool(true));

    let totals = field(&report, "totals");
    assert_eq!(py_str(field(totals, "quantityMsat")), "150000000000");
    assert_eq!(py_str(field(totals, "altQuantityMsat")), "100000000000");
    assert_eq!(py_str(field(totals, "neuQuantityMsat")), "50000000000");
    // Total cost basis: 8500 + 32000 = 40500
    assert!((num(field(totals, "costBasis")) - 40500.0).abs() < 1e-6);
    // Market value at 58,000 EUR/BTC: 1.5 * 58000 = 87000
    assert!((num(field(totals, "marketValue")) - 87000.0).abs() < 1e-6);
    // Niederstwert: Alt min(8500, 58000)=8500 + Neu min(32000, 29000)=29000 -> 37500
    assert!((num(field(totals, "lowerOfCostOrMarket")) - 37500.0).abs() < 1e-6);

    let assets = list(field(&report, "assets"));
    assert_eq!(assets.len(), 2);
    let alt = &assets[0];
    let neu = &assets[1];
    assert_eq!(py_str(field(alt, "regime")), "alt");
    assert!((num(field(alt, "costBasis")) - 8500.0).abs() < 1e-6);
    assert!((num(field(alt, "lowerOfCostOrMarket")) - 8500.0).abs() < 1e-6);
    assert!((num(field(alt, "impairmentIndicator")) - 0.0).abs() < 1e-6);
    assert_eq!(py_str(field(neu, "regime")), "neu");
    assert!((num(field(neu, "costBasis")) - 32000.0).abs() < 1e-6);
    assert!((num(field(neu, "marketValue")) - 29000.0).abs() < 1e-6);
    assert!((num(field(neu, "lowerOfCostOrMarket")) - 29000.0).abs() < 1e-6);
    assert!((num(field(neu, "impairmentIndicator")) - 3000.0).abs() < 1e-6);

    // Under global pool, per-wallet cost basis is marked as pro-rata allocated.
    let wallets = list(field(&report, "walletHoldings"));
    assert_eq!(wallets.len(), 2);
    assert!(wallets
        .iter()
        .all(|w| field(w, "costBasisAllocated") == &Value::Bool(true)));

    // Render PDF, XLSX and CSV ZIP and verify layout + live MIN formula.
    let model = kassiber_reports::wealth::model(conn, &args, &Fixed).unwrap();
    let doc = report_design::wealth(&model);
    assert!(doc.watermark.is_none());
    let rendered = report_design::rendered_pdf(&doc).unwrap();
    assert!(
        rendered.off_page.is_empty(),
        "final wealth PDF overflow: {:?}",
        rendered.off_page
    );
    assert!(!rendered.pages.is_empty());

    let xlsx = report_design::xlsx_document(&doc);
    let xlsx_json = serde_json::to_string(&xlsx).unwrap();
    assert!(
        xlsx_json.contains("MIN("),
        "XLSX should include live MIN formula for lower-of-cost-or-market"
    );

    let zip = report_design::csv_zip(&doc).unwrap();
    assert!(!zip.is_empty());

    // Toggling include_regime = false merges into a single 'all' row with identical totals.
    let no_regime = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
        ("include_regime", Value::Bool(false)),
    ]);
    let merged =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &no_regime, &Fixed).unwrap();
    let merged_assets = list(field(&merged, "assets"));
    assert_eq!(merged_assets.len(), 1);
    assert_eq!(py_str(field(&merged_assets[0], "regime")), "all");
    assert!((num(field(&merged_assets[0], "costBasis")) - 40500.0).abs() < 1e-6);

    // Toggling include_wallets = false omits walletHoldings and the holdings_by_wallet section.
    let no_wallets = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
        ("include_wallets", Value::Bool(false)),
    ]);
    let no_wallets_model = kassiber_reports::wealth::model(conn, &no_wallets, &Fixed).unwrap();
    let no_wallets_report = field(&no_wallets_model, "report");
    assert_eq!(
        field(no_wallets_report, "includeWallets"),
        &Value::Bool(false)
    );
    assert!(list(field(no_wallets_report, "walletHoldings")).is_empty());
    let no_wallets_doc = report_design::wealth(&no_wallets_model);
    assert!(
        !no_wallets_doc
            .sections
            .iter()
            .any(|s| s.id == "holdings_by_wallet"),
        "holdings_by_wallet section must be omitted when include_wallets=false"
    );
}

#[test]
fn per_wallet_cost_basis_pool_reports_exact_wallet_basis() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    conn.execute_batch(
        r#"
        INSERT INTO workspaces(id, label, created_at)
        VALUES ('ws-1', 'Work', '2023-01-01T00:00:00Z');
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            gains_algorithm, cost_basis_pool_scope,
            last_processed_at, last_processed_tx_count,
            journal_input_version, last_processed_input_version,
            created_at
        ) VALUES (
            'prof-de', 'ws-1', 'Depot DE', 'EUR', 'de',
            'fifo', 'wallet',
            '2026-01-05T10:00:00Z', 2,
            2, 2,
            '2023-01-01T00:00:00Z'
        );
        INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
        VALUES
            ('w-a', 'ws-1', 'prof-de', 'Vault A', 'descriptor', '{"chain":"bitcoin","network":"mainnet"}', '2023-01-01T00:00:00Z'),
            ('w-b', 'ws-1', 'prof-de', 'Vault B', 'descriptor', '{"chain":"bitcoin","network":"mainnet"}', '2024-01-01T00:00:00Z');
        INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at, direction, asset, amount, fee, fiat_value, excluded, created_at)
        VALUES
            ('t1', 'ws-1', 'prof-de', 'w-a', 'fp-1', '2023-06-01T10:00:00Z', 'inbound', 'BTC', 100000000000, 0, 20000.0, 0, '2023-06-01T10:00:00Z'),
            ('t2', 'ws-1', 'prof-de', 'w-b', 'fp-2', '2024-06-01T10:00:00Z', 'inbound', 'BTC', 100000000000, 0, 50000.0, 0, '2024-06-01T00:00:00Z');
        INSERT INTO journal_entries(id, workspace_id, profile_id, wallet_id, transaction_id, occurred_at, entry_type, asset, quantity, fiat_value, cost_basis, created_at)
        VALUES
            ('j1', 'ws-1', 'prof-de', 'w-a', 't1', '2023-06-01T10:00:00Z', 'acquisition', 'BTC', 100000000000, 20000.0, 20000.0, '2026-01-05T10:00:00Z'),
            ('j2', 'ws-1', 'prof-de', 'w-b', 't2', '2024-06-01T10:00:00Z', 'acquisition', 'BTC', 100000000000, 50000.0, 50000.0, '2026-01-05T10:00:00Z');
        INSERT INTO rates_cache(pair, timestamp, rate, rate_exact, source, fetched_at)
        VALUES ('BTC-EUR', '2025-12-31T23:55:00Z', 60000.0, '60000', 'kraken_csv', '2026-01-01T00:00:00Z');
        "#,
    )
    .unwrap();

    let args = dict([
        ("workspace", s("Work")),
        ("profile", s("Depot DE")),
        ("as_of", s("2025-12-31")),
    ]);
    let report =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &args, &Fixed).unwrap();
    assert_eq!(py_str(field(&report, "costBasisPoolScope")), "wallet");
    let wallets = list(field(&report, "walletHoldings"));
    assert_eq!(wallets.len(), 2);
    assert_eq!(py_str(field(&wallets[0], "wallet")), "Vault A");
    assert!((num(field(&wallets[0], "costBasis")) - 20000.0).abs() < 1e-6);
    assert_eq!(
        field(&wallets[0], "costBasisAllocated"),
        &Value::Bool(false)
    );
    assert_eq!(py_str(field(&wallets[1], "wallet")), "Vault B");
    assert!((num(field(&wallets[1], "costBasis")) - 50000.0).abs() < 1e-6);
    assert_eq!(
        field(&wallets[1], "costBasisAllocated"),
        &Value::Bool(false)
    );
}

#[test]
fn stale_journals_fall_back_to_watermarked_draft_without_blocking() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);
    // Make journals stale so normal reports would block.
    conn.execute(
        "UPDATE profiles SET journal_input_version = 9, last_processed_input_version = 5 WHERE id = 'prof-at'",
        [],
    )
    .unwrap();

    let args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
    ]);
    let report =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &args, &Fixed).unwrap();
    assert_eq!(py_str(field(&report, "status")), "draft");
    assert_eq!(field(&report, "isDraft"), &Value::Bool(true));
    assert_eq!(py_str(field(&report, "dataSource")), "observed");
    // Exact msat quantity still matches 1.5 BTC, but cost basis is withheld fail-closed!
    assert_eq!(
        py_str(field(field(&report, "totals"), "quantityMsat")),
        "150000000000"
    );
    assert_eq!(field(field(&report, "totals"), "costBasis"), &Value::Null);
    let assets = list(field(&report, "assets"));
    assert_eq!(assets.len(), 1);
    assert_eq!(py_str(field(&assets[0], "regime")), "unresolved");

    // Draft PDF carries a watermark and renders cleanly with zero overflow.
    let model = kassiber_reports::wealth::model(conn, &args, &Fixed).unwrap();
    let doc = report_design::wealth(&model);
    assert_eq!(doc.watermark.as_deref(), Some("ENTWURF"));
    let rendered = report_design::rendered_pdf(&doc).unwrap();
    assert!(
        rendered.off_page.is_empty(),
        "draft wealth PDF overflow: {:?}",
        rendered.off_page
    );
}

#[test]
fn missing_cutoff_rate_fails_closed_for_market_value() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);

    // 2024-12-31 has no FX quote in the DB -> marketValue & lowerOfCostOrMarket stay null!
    let args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2024-12-31")),
    ]);
    let report =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &args, &Fixed).unwrap();
    let totals = field(&report, "totals");
    assert!((num(field(totals, "costBasis")) - 40500.0).abs() < 1e-6);
    assert_eq!(field(totals, "marketValue"), &Value::Null);
    assert_eq!(field(totals, "lowerOfCostOrMarket"), &Value::Null);
    assert_eq!(field(totals, "unrealizedPnl"), &Value::Null);
    assert!(!list(field(&report, "missingFmvAssets")).is_empty());
}

#[test]
fn blocked_year_handoff_refuses_final_and_succeeds_with_draft_flag() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);
    // Quarantine tx-neu -> blocks final year handoff for 2024.
    conn.execute(
        "INSERT INTO journal_quarantines(transaction_id, workspace_id, profile_id, reason, created_at) VALUES ('tx-neu', 'ws-1', 'prof-at', 'missing_price', '2026-02-01T10:00:00Z')",
        [],
    )
    .unwrap();

    let out_final = dir.path().join("handoff-final.xlsx");
    let final_args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("year", Value::Int(2024.into())),
        ("file", s(out_final.to_str().unwrap())),
    ]);
    let err = kassiber_reports::export_request::execute(
        conn,
        "reports.export-year-handoff",
        &final_args,
        dir.path(),
        &Fixed,
    )
    .unwrap_err();
    assert_eq!(err.code, "year_handoff_quarantined");

    let out_draft = dir.path().join("handoff-draft.xlsx");
    let draft_args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("year", Value::Int(2024.into())),
        ("file", s(out_draft.to_str().unwrap())),
        ("draft", Value::Bool(true)),
    ]);
    let ok = kassiber_reports::export_request::execute(
        conn,
        "reports.export-year-handoff",
        &draft_args,
        dir.path(),
        &Fixed,
    )
    .unwrap();
    assert!(out_draft.is_file());
    assert!(!list(field(&ok, "sheets")).is_empty());

    // Draft exports must never register a filed_report_snapshots record.
    let snap_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(snap_count, 0);
}

#[test]
fn final_wealth_statement_uses_finanzamt_safe_wording_and_portrait_layout() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);

    let args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
    ]);
    let model = kassiber_reports::wealth::model(conn, &args, &Fixed).unwrap();
    let doc = report_design::wealth(&model);

    // Uniform Portrait layout: zero landscape sections, zero off-page overflow.
    assert!(doc.sections.iter().all(|s| !s.landscape));
    let rendered = report_design::rendered_pdf(&doc).unwrap();
    assert!(
        rendered.off_page.is_empty(),
        "portrait wealth PDF overflow: {:?}",
        rendered.off_page
    );

    // Final report omits the "issues" section entirely and never leaks technical/tentative wording.
    assert!(
        !doc.sections.iter().any(|s| s.id == "issues"),
        "clean final report must not render an Open Issues / Draft Notes section"
    );
    let dumped = format!("{doc:?}");
    for forbidden in [
        "descriptor",
        "Offene Punkte",
        "Entwurf",
        "obliegt der Steuerberatung",
        "kein Nachweis physischer Steuerlose",
    ] {
        assert!(
            !dumped.contains(forbidden),
            "final wealth document must not contain '{forbidden}'"
        );
    }
    assert!(
        dumped.contains("Kraken (Marktdaten)"),
        "concrete rate provider should be shown instead of generic cache label"
    );
    assert!(
        dumped.contains("Abwertungsbedarf: 3.000,00 EUR"),
        "KPI detail should surface Niederstwert write-down"
    );
}

#[test]
fn draft_wealth_statement_renders_localized_german_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);
    conn.execute(
        "UPDATE profiles SET journal_input_version = 9, last_processed_input_version = 5 WHERE id = 'prof-at'",
        [],
    )
    .unwrap();

    let args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
    ]);
    let model = kassiber_reports::wealth::model(conn, &args, &Fixed).unwrap();
    let doc = report_design::wealth(&model);
    let dumped = format!("{doc:?}");
    assert!(
        dumped.contains("Journale wurden nach den letzten Transaktionen"),
        "German draft wealth PDF must localize draft reasons in German"
    );
    assert!(
        !dumped.contains("Journals are not up to date"),
        "German draft wealth PDF must not contain raw English draft reason strings"
    );
}

#[test]
fn privacy_mode_redacts_wallets_and_txids_across_wealth_e1kv_and_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);
    // Add a 2025 disposal with an external txid and select the active book for ui.* exports.
    conn.execute_batch(
        r#"
        INSERT OR REPLACE INTO settings(key, value) VALUES
            ('context_workspace', 'ws-1'),
            ('context_profile', 'prof-at'),
            ('database_instance_id', '0123456789abcdef0123456789abcdef');
        UPDATE profiles SET last_processed_tx_count = 4 WHERE id = 'prof-at';
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, external_id, occurred_at,
            direction, asset, amount, fee, fiat_value, excluded, created_at
        ) VALUES (
            'tx-sell-2025', 'ws-1', 'prof-at', 'w-hot', 'fp-sell',
            'deadbeef1234567890abcdef1234567890abcdef1234567890abcdef12345678',
            '2025-07-01T12:00:00Z', 'outbound', 'BTC',
            10000000000, 0, 8000.0, 0, '2025-07-01T12:00:00Z'
        );
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, wallet_id, transaction_id, occurred_at,
            entry_type, asset, quantity, fiat_value, proceeds, cost_basis, gain_loss,
            at_category, at_kennzahl, created_at
        ) VALUES (
            'je-sell-2025', 'ws-1', 'prof-at', 'w-hot', 'tx-sell-2025', '2025-07-01T12:00:00Z',
            'disposal', 'BTC', -10000000000, 8000.0, 8000.0, 6400.0, 1600.0,
            'neu_gain', 174, '2026-02-01T10:00:00Z'
        );
        "#,
    )
    .unwrap();

    // 1. Wealth statement in privacy_mode groups all self-custody wallets into "Eigenverwahrung".
    let wealth_args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
        ("privacy_mode", Value::Bool(true)),
    ]);
    let w_model = kassiber_reports::wealth::model(conn, &wealth_args, &Fixed).unwrap();
    let mut w_doc = report_design::wealth(&w_model);
    w_doc.apply_privacy_mode();
    assert!(w_doc.privacy_mode);
    let w_rendered = report_design::rendered_pdf(&w_doc).unwrap();
    assert!(
        w_rendered.off_page.is_empty(),
        "privacy wealth PDF overflow: {:?}",
        w_rendered.off_page
    );
    let w_dump = format!("{w_doc:?}");
    assert!(w_dump.contains("Eigenverwahrung"));
    assert!(!w_dump.contains("Coldcard"));
    assert!(!w_dump.contains("Sparrow"));
    assert!(!w_dump.contains("descriptor"));
    let w_wallets = list(field(field(&w_model, "report"), "walletHoldings"));
    assert_eq!(
        w_wallets.len(),
        1,
        "both self-custody wallets must aggregate into a single Eigenverwahrung row"
    );

    // 2. Austrian E 1kv in privacy_mode omits transaction reference / txid & replaces wallet names.
    let e1kv_args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("year", Value::Int(2025.into())),
        ("privacy_mode", Value::Bool(true)),
    ]);
    let e1kv_out = kassiber_reports::export_request::execute(
        conn,
        "ui.reports.export_austrian_e1kv_csv",
        &e1kv_args,
        dir.path(),
        &Fixed,
    )
    .unwrap();
    let csv_dir = std::path::PathBuf::from(py_str(field(&e1kv_out, "dir")));
    let tx_csv = std::fs::read_to_string(csv_dir.join("transactions.csv")).unwrap();
    assert!(
        !tx_csv.contains("reference"),
        "transactions.csv must omit reference column in privacy_mode"
    );
    assert!(
        !tx_csv.contains("deadbeef"),
        "transactions.csv must not contain txids in privacy_mode"
    );
    assert!(
        !tx_csv.contains("Sparrow") && !tx_csv.contains("Coldcard"),
        "transactions.csv must not leak wallet names in privacy_mode"
    );
    assert!(tx_csv.contains("Eigenverwahrung"));
    let manifest = std::fs::read_to_string(csv_dir.join("manifest.json")).unwrap();
    assert!(manifest.contains("\"privacy_mode\": true"));
    let e1kv_pdf_out = kassiber_reports::export_request::execute(
        conn,
        "ui.reports.export_austrian_e1kv_pdf",
        &e1kv_args,
        dir.path(),
        &Fixed,
    )
    .unwrap();
    assert!(py_str(field(&e1kv_pdf_out, "filename")).contains("-minimal-"));
    assert!(std::fs::read(py_str(field(&e1kv_pdf_out, "file")))
        .unwrap()
        .starts_with(b"%PDF-"));

    // 3. Year handoff XLSX in privacy_mode drops wallets sheet & txid columns, keeps formulas.
    let yh_args = dict([
        ("year", Value::Int(2025.into())),
        ("privacy_mode", Value::Bool(true)),
    ]);
    let yh_out = kassiber_reports::export_request::execute(
        conn,
        "ui.reports.export_year_handoff",
        &yh_args,
        dir.path(),
        &Fixed,
    )
    .unwrap();
    let sheets: Vec<String> = list(field(&yh_out, "sheets")).iter().map(py_str).collect();
    assert!(
        !sheets.contains(&"wallets".to_owned()),
        "privacy_mode must drop raw wallets inventory sheet: {sheets:?}"
    );
    assert!(std::fs::read(py_str(field(&yh_out, "file")))
        .unwrap()
        .starts_with(b"PK"));
}

#[test]
fn observed_fallback_respects_amount_includes_fee_and_quarantines() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);
    // Add an outbound fee-inclusive tx (0.2 BTC total incl. 0.001 BTC fee) and quarantine it.
    conn.execute_batch(
        r#"
        UPDATE profiles SET last_processed_tx_count = 4 WHERE id = 'prof-at';
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at,
            direction, asset, amount, fee, amount_includes_fee, fiat_value, excluded, created_at
        ) VALUES (
            'tx-fee-inc', 'ws-1', 'prof-at', 'w-hot', 'fp-fee-inc',
            '2025-08-01T12:00:00Z', 'outbound', 'BTC',
            20000000000, 100000000, 1, 16000.0, 0, '2025-08-01T12:00:00Z'
        );
        INSERT INTO journal_quarantines(
            transaction_id, workspace_id, profile_id, reason, detail_json, created_at
        ) VALUES (
            'tx-fee-inc', 'ws-1', 'prof-at', 'missing_price', '{}', '2026-02-01T10:00:00Z'
        );
        "#,
    )
    .unwrap();

    let args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
    ]);
    let report =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &args, &Fixed).unwrap();
    // Quarantine before cutoff must force observed fallback instead of dropping quarantined coins.
    assert_eq!(field(&report, "isDraft"), &Value::Bool(true));
    assert_eq!(py_str(field(&report, "dataSource")), "observed");
    assert_eq!(field(&report, "regimeResolved"), &Value::Bool(false));
    assert_eq!(field(field(&report, "totals"), "costBasis"), &Value::Null);
    // 1.5 BTC - 0.2 BTC (fee included in amount, NOT double-subtracted!) = 1.3 BTC = 130_000_000_000 msat.
    assert_eq!(
        py_str(field(field(&report, "totals"), "quantityMsat")),
        "130000000000"
    );
}

#[test]
fn manual_quote_within_24h_beats_newer_market_quote() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);
    // Seed a manual cutoff quote at 12:00 alongside the newer 22:30 kraken_csv quote.
    conn.execute_batch(
        r#"
        INSERT INTO rates_cache(pair, timestamp, rate, rate_exact, source, fetched_at)
        VALUES ('BTC-EUR', '2025-12-31T12:00:00Z', 75000.1234, '75000.1234', 'manual', '2026-01-01T00:00:00Z');
        "#,
    )
    .unwrap();

    let args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
    ]);
    let report =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &args, &Fixed).unwrap();
    let sources = list(field(&report, "valuationSources"));
    assert_eq!(sources.len(), 1);
    assert_eq!(py_str(field(&sources[0], "provider")), "manual");
    assert_eq!(py_str(field(&sources[0], "rateExact")), "75000.1234");
}

#[test]
fn privacy_mode_skips_full_xlsx_verification_and_redacts_capital_csv_and_platform_wallets() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);
    conn.execute_batch(
        r#"
        INSERT OR REPLACE INTO settings(key, value) VALUES
            ('context_workspace', 'ws-1'),
            ('context_profile', 'prof-at'),
            ('database_instance_id', '0123456789abcdef0123456789abcdef');
        INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
        VALUES ('w-ex', 'ws-1', 'prof-at', 'Kraken Main', 'exchange', '{"chain":"bitcoin","network":"mainnet"}', '2024-01-01T00:00:00Z');
        UPDATE profiles SET last_processed_tx_count = 4 WHERE id = 'prof-at';
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, external_id, occurred_at,
            direction, asset, amount, fee, fiat_value, excluded, created_at
        ) VALUES (
            'tx-ex-sell', 'ws-1', 'prof-at', 'w-ex', 'fp-ex-sell',
            'cafebabe1234567890abcdef1234567890abcdef1234567890abcdef12345678',
            '2025-07-01T12:00:00Z', 'outbound', 'BTC',
            10000000000, 0, 8000.0, 0, '2025-07-01T12:00:00Z'
        );
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, wallet_id, transaction_id, occurred_at,
            entry_type, asset, quantity, fiat_value, proceeds, cost_basis, gain_loss,
            at_category, at_kennzahl, created_at
        ) VALUES (
            'je-ex-sell', 'ws-1', 'prof-at', 'w-ex', 'tx-ex-sell', '2025-07-01T12:00:00Z',
            'disposal', 'BTC', -10000000000, 8000.0, 8000.0, 6400.0, 1600.0,
            'neu_gain', 174, '2026-02-01T10:00:00Z'
        );
        "#,
    )
    .unwrap();

    // 1. Full XLSX in privacy_mode omits verification appendix (Verify/Acquisitions/Disposals/Control).
    let xlsx_out = kassiber_reports::export_request::execute(
        conn,
        "ui.reports.export_xlsx",
        &dict([("privacy_mode", Value::Bool(true))]),
        dir.path(),
        &Fixed,
    )
    .unwrap();
    let sheets: Vec<String> = list(field(&xlsx_out, "sheets"))
        .iter()
        .map(py_str)
        .collect();
    assert!(
        !sheets.contains(&"Verify".to_owned()) && !sheets.contains(&"Acquisitions".to_owned()),
        "privacy_mode must skip verification appendix sheets: {sheets:?}"
    );

    // 2. Capital gains CSV in privacy_mode strips transaction_id and maps exchange wallet to platform.
    let cg_out = kassiber_reports::export_request::execute(
        conn,
        "ui.reports.export_capital_gains_csv",
        &dict([
            ("year", Value::Int(2025.into())),
            ("privacy_mode", Value::Bool(true)),
        ]),
        dir.path(),
        &Fixed,
    )
    .unwrap();
    let cg_csv = std::fs::read_to_string(py_str(field(&cg_out, "file"))).unwrap();
    assert!(!cg_csv.contains("transaction_id"));
    assert!(!cg_csv.contains("Kraken Main"));
    assert!(!cg_csv.contains("tx-ex-sell"));
    assert!(cg_csv.contains(",platform,"));

    // 3. Austrian E 1kv in privacy_mode labels exchange rows as Handelsplattform (not Eigenverwahrung).
    let e1kv_out = kassiber_reports::export_request::execute(
        conn,
        "ui.reports.export_austrian_e1kv_csv",
        &dict([
            ("year", Value::Int(2025.into())),
            ("privacy_mode", Value::Bool(true)),
        ]),
        dir.path(),
        &Fixed,
    )
    .unwrap();
    let csv_dir = std::path::PathBuf::from(py_str(field(&e1kv_out, "dir")));
    let disp_csv = std::fs::read_to_string(csv_dir.join("disposals.csv")).unwrap();
    assert!(!disp_csv.contains("Kraken Main"));
    assert!(
        disp_csv.contains("Handelsplattform"),
        "exchange wallet must map to Handelsplattform in privacy_mode: {disp_csv}"
    );
    // Descriptions (which can carry `Transfer <from> -> <to>` or raw row IDs) must be omitted.
    assert!(
        !cg_csv.contains("description"),
        "capital_csv must omit description column in privacy_mode: {cg_csv}"
    );
    let tx_csv = std::fs::read_to_string(csv_dir.join("transactions.csv")).unwrap();
    assert!(
        !tx_csv.lines().next().unwrap_or("").contains("description"),
        "transactions.csv must omit description column in privacy_mode: {tx_csv}"
    );
}

#[test]
fn wallet_filtered_global_pool_respects_book_wide_quarantines() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);
    // Add a quarantine in `w-hot` (Sparrow), then query wealth filtered to `w-cold` (Coldcard).
    conn.execute_batch(
        r#"
        UPDATE profiles SET last_processed_tx_count = 4 WHERE id = 'prof-at';
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at,
            direction, asset, amount, fee, fiat_value, excluded, created_at
        ) VALUES (
            'tx-unselected-q', 'ws-1', 'prof-at', 'w-hot', 'fp-unselected-q',
            '2025-05-01T12:00:00Z', 'inbound', 'BTC',
            10000000000, 0, NULL, 0, '2025-05-01T12:00:00Z'
        );
        INSERT INTO journal_quarantines(
            transaction_id, workspace_id, profile_id, reason, detail_json, created_at
        ) VALUES (
            'tx-unselected-q', 'ws-1', 'prof-at', 'missing_price', '{}', '2026-02-01T10:00:00Z'
        );
        "#,
    )
    .unwrap();

    let args = dict([
        ("workspace", s("Privat")),
        ("profile", s("Hauptbuch AT")),
        ("as_of", s("2025-12-31")),
        ("wallets", Value::List(vec![s("w-cold")])),
    ]);
    let report =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &args, &Fixed).unwrap();
    assert_eq!(field(&report, "isDraft"), &Value::Bool(true));
    assert_eq!(py_str(field(&report, "status")), "draft");
    assert_eq!(py_str(field(&report, "dataSource")), "observed");
    assert_eq!(
        field(field(&report, "totals"), "costBasis"),
        &Value::Null,
        "pooled basis must be withheld when an unselected wallet has a quarantine before cutoff"
    );
}

#[test]
fn capital_gains_csv_refuses_draft_mode() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    seed_austrian_book(conn);
    conn.execute_batch(
        r#"
        INSERT OR REPLACE INTO settings(key, value) VALUES
            ('context_workspace', 'ws-1'),
            ('context_profile', 'prof-at'),
            ('database_instance_id', '0123456789abcdef0123456789abcdef');
        "#,
    )
    .unwrap();
    let err = kassiber_reports::export_request::execute(
        conn,
        "ui.reports.export_capital_gains_csv",
        &dict([
            ("year", Value::Int(2025.into())),
            ("draft", Value::Bool(true)),
        ]),
        dir.path(),
        &Fixed,
    )
    .unwrap_err();
    assert_eq!(err.code, "validation");
}

#[test]
fn us_global_then_wallet_post_2025_reports_exact_wallet_basis() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(&dir.path().join("data"), None).unwrap();
    let conn = book.connection();
    conn.execute_batch(
        r#"
        INSERT INTO workspaces(id, label, created_at) VALUES ('ws-us', 'US', '2025-01-01T00:00:00Z');
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            gains_algorithm, cost_basis_pool_scope,
            last_processed_at, last_processed_tx_count,
            journal_input_version, last_processed_input_version,
            created_at
        ) VALUES (
            'prof-us', 'ws-us', 'US Main', 'USD', 'us',
            'fifo', 'global_then_wallet',
            '2026-01-05T10:00:00Z', 2,
            2, 2,
            '2025-01-01T00:00:00Z'
        );
        INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
        VALUES
            ('w-1', 'ws-us', 'prof-us', 'Wallet 1', 'descriptor', '{"chain":"bitcoin","network":"mainnet"}', '2025-01-01T00:00:00Z'),
            ('w-2', 'ws-us', 'prof-us', 'Wallet 2', 'descriptor', '{"chain":"bitcoin","network":"mainnet"}', '2025-01-01T00:00:00Z');
        INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at, direction, asset, amount, fee, fiat_value, excluded, created_at)
        VALUES
            ('t1', 'ws-us', 'prof-us', 'w-1', 'fp-us-1', '2025-03-01T10:00:00Z', 'inbound', 'BTC', 100000000000, 0, 10000.0, 0, '2025-03-01T10:00:00Z'),
            ('t2', 'ws-us', 'prof-us', 'w-2', 'fp-us-2', '2025-04-01T10:00:00Z', 'inbound', 'BTC', 100000000000, 0, 90000.0, 0, '2025-04-01T10:00:00Z');
        INSERT INTO journal_entries(id, workspace_id, profile_id, wallet_id, transaction_id, occurred_at, entry_type, asset, quantity, fiat_value, cost_basis, created_at)
        VALUES
            ('j1', 'ws-us', 'prof-us', 'w-1', 't1', '2025-03-01T10:00:00Z', 'acquisition', 'BTC', 100000000000, 10000.0, 10000.0, '2026-01-05T10:00:00Z'),
            ('j2', 'ws-us', 'prof-us', 'w-2', 't2', '2025-04-01T10:00:00Z', 'acquisition', 'BTC', 100000000000, 90000.0, 90000.0, '2026-01-05T10:00:00Z');
        INSERT INTO rates_cache(pair, timestamp, rate, rate_exact, source, fetched_at)
        VALUES ('BTC-USD', '2025-12-31T23:55:00Z', 80000.0, '80000', 'kraken_csv', '2026-01-01T00:00:00Z');
        "#,
    )
    .unwrap();

    let all_args = dict([
        ("workspace", s("US")),
        ("profile", s("US Main")),
        ("as_of", s("2025-12-31")),
    ]);
    let all =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &all_args, &Fixed).unwrap();
    assert_eq!(field(&all, "perWalletBasis"), &Value::Bool(true));
    let wallets = list(field(&all, "walletHoldings"));
    assert_eq!(wallets.len(), 2);
    assert!((num(field(&wallets[0], "costBasis")) - 10000.0).abs() < 1e-6);
    assert_eq!(
        field(&wallets[0], "costBasisAllocated"),
        &Value::Bool(false)
    );
    assert!((num(field(&wallets[1], "costBasis")) - 90000.0).abs() < 1e-6);
    assert_eq!(
        field(&wallets[1], "costBasisAllocated"),
        &Value::Bool(false)
    );

    // Filtering to Wallet 1 must report $10,000, not $50,000 pro-rata.
    let one_args = dict([
        ("workspace", s("US")),
        ("profile", s("US Main")),
        ("as_of", s("2025-12-31")),
        ("wallets", Value::List(vec![s("Wallet 1")])),
    ]);
    let one =
        kassiber_reports::execute_with_sources(conn, "reports.wealth", &one_args, &Fixed).unwrap();
    assert!((num(field(field(&one, "totals"), "costBasis")) - 10000.0).abs() < 1e-6);
}
