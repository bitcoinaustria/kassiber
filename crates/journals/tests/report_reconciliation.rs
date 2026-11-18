//! The figures every report reads from one processed journal agree: the
//! review, the summary PDF and the country forms. Each case starts from raw
//! transactions in a disposable book and runs the production journal.
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::Value;
use kassiber_reports::{ch_tax, gb_tax, jp_tax, review_figures, summary_pdf, tax_years};
use kassiber_store::{rusqlite::params, Book};

const BTC: i64 = 100_000_000_000;

struct Case {
    book: Book,
    root: std::path::PathBuf,
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Case {
    fn new(country: &str, currency: &str, method: &str, pools: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "report-reconciliation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let book = Book::create(&root, None).unwrap();
        let conn = book.connection();
        conn.execute_batch(
            "INSERT INTO workspaces(id,label,created_at) VALUES ('ws','Main','2020');
             INSERT INTO settings(key,value) VALUES ('context_workspace','ws'),('context_profile','p')
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO profiles(id,workspace_id,label,tax_country,fiat_currency,gains_algorithm,cost_basis_pool_scope,created_at)
             VALUES ('p','ws','Book',?1,?2,?3,?4,'2020')",
            params![country, currency, method, pools],
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at)
                 VALUES ('a','ws','p','fund','Fund','asset','2020');
             INSERT INTO wallets(id,workspace_id,profile_id,account_id,label,kind,config_json,created_at)
                 VALUES ('w','ws','p','a','Cold','custom','{}','2020'),
                        ('b','ws','p','a','Hot','custom','{}','2020');",
        )
        .unwrap();
        Self { book, root }
    }

    /// A transaction priced by its rate only, as a priced import stores it.
    fn tx(&self, id: &str, wallet: &str, kind: &str, at: &str, btc: i64, rate: &str) {
        self.book
            .connection()
            .execute(
                "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,
                     fiat_rate,fiat_rate_exact,fiat_currency,excluded,occurred_at,created_at,kind)
                 VALUES (?1,'ws','p',?2,?1,'BTC',?3,?4,0,CAST(?5 AS REAL),?5,
                     (SELECT fiat_currency FROM profiles WHERE id='p'),0,?6,?6,?7)",
                params![
                    id,
                    wallet,
                    if kind == "sell" { "outbound" } else { "inbound" },
                    btc,
                    rate,
                    at,
                    kind
                ],
            )
            .unwrap();
    }

    fn sql(&self, sql: &str) {
        self.book.connection().execute_batch(sql).unwrap();
    }

    fn process(&self) {
        let conn = self.book.connection();
        kassiber_journals::service::process(conn, None, None, &mut |_| {})
            .unwrap()
            .after_reply()
            .unwrap();
        let held: i64 = conn
            .query_row("SELECT COUNT(*) FROM journal_quarantines", [], |r| r.get(0))
            .unwrap();
        assert_eq!(held, 0, "every case books fully");
    }

    fn profile(&self) -> (Value, Value) {
        kassiber_setup::references::scope(self.book.connection(), &dict::<&str>([])).unwrap()
    }

    fn review(&self, year: i64) -> f64 {
        let cut = tax_years::Cut::of(self.book.connection(), &self.profile().1).unwrap();
        let totals = review_figures::year_totals(self.book.connection(), "p", cut).unwrap();
        totals
            .get(&year)
            .map_or(0., |t| t[0].to_string().parse().unwrap())
    }

    fn summary(&self, args: Value) -> Value {
        let (workspace, profile) = self.profile();
        summary_pdf::model(
            self.book.connection(),
            &workspace,
            &profile,
            &args,
            &kassiber_setup::SystemSources,
        )
        .unwrap()
    }
}

fn number(value: &Value) -> f64 {
    match value {
        Value::Float(f) => *f,
        Value::Int(n) => n.to_string().parse().unwrap(),
        Value::Str(_) => kassiber_envelope::py_str(value).parse().unwrap(),
        other => panic!("not a number: {other:?}"),
    }
}

fn at<'a>(value: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(value, |v, key| {
        get(v, key).unwrap_or_else(|| panic!("{key}"))
    })
}

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        other => panic!("not a list: {other:?}"),
    }
}

#[test]
fn uk_fee_units_realize_in_the_review_and_summary_as_on_sa108() {
    let case = Case::new("gb", "GBP", "UK_SHARE_MATCHING", "global");
    case.tx("buy", "w", "buy", "2024-05-01T12:00:00Z", 2 * BTC, "100");
    case.tx("sell", "w", "sell", "2024-07-10T12:00:00Z", BTC, "400");
    case.sql("UPDATE transactions SET fee=10000000000,amount_includes_fee=0 WHERE id='sell'");
    case.process();
    // 1 BTC sold at 400 and 0.1 BTC of fee units at 40, both from a pool at
    // 100: 300 on the sale less the fee's 40, plus 30 on the fee units.
    let model = gb_tax::model(case.book.connection(), &case.profile().1, None).unwrap();
    let form = number(at(&list(at(&model, &["years"]))[0], &["net_gains"]));
    assert_eq!(form, 290.);
    assert_eq!(case.review(2024), form);
    let summary = case.summary(dict([("year", 2024.into())]));
    assert_eq!(number(at(&summary, &["metrics", "realized_pnl"])), form);
}

#[test]
fn generic_fee_units_add_nothing_to_the_review() {
    let case = Case::new("generic", "EUR", "FIFO", "global");
    case.tx("buy", "w", "buy", "2024-05-01T12:00:00Z", 2 * BTC, "100");
    case.tx("sell", "w", "sell", "2024-07-10T12:00:00Z", BTC, "400");
    case.sql("UPDATE transactions SET fee=10000000000,amount_includes_fee=0 WHERE id='sell'");
    case.process();
    let summary = case.summary(dict([("year", 2024.into())]));
    assert_eq!(
        case.review(2024),
        number(at(&summary, &["metrics", "realized_pnl"]))
    );
}

#[test]
fn summary_bounds_are_the_holdings_before_and_at_the_period() {
    let case = Case::new("generic", "EUR", "FIFO", "global");
    case.tx("before", "w", "buy", "2023-11-01T12:00:00Z", BTC, "50");
    case.tx("buy", "w", "buy", "2024-05-01T12:00:00Z", BTC, "100");
    case.tx("sell", "w", "sell", "2024-07-10T12:00:00Z", 2 * BTC, "400");
    case.process();
    let summary = case.summary(dict([("year", 2024.into())]));
    let metrics = at(&summary, &["metrics"]);
    // One coin before the year, none at its end, though the history's last
    // month with a holding is June.
    assert_eq!(number(at(metrics, &["btc_stack_start"])), 1.);
    for key in ["btc_stack_end", "period_end_value", "end_cost_basis"] {
        assert_eq!(number(at(metrics, &[key])), 0., "{key}");
    }
}

#[test]
fn a_uk_summary_closes_on_april_fifth() {
    let case = Case::new("gb", "GBP", "UK_SHARE_MATCHING", "global");
    case.tx("held", "w", "buy", "2024-06-01T12:00:00Z", BTC, "100");
    case.tx("later", "w", "buy", "2025-04-20T12:00:00Z", 4 * BTC, "200");
    case.process();
    let summary = case.summary(dict([("year", 2024.into())]));
    assert_eq!(number(at(&summary, &["metrics", "btc_stack_end"])), 1.);
    assert_eq!(
        number(at(&summary, &["holdings_totals", "total_quantity"])),
        1.
    );
}

#[test]
fn a_wallet_scoped_summary_keeps_its_wallet_pool_basis() {
    let case = Case::new("us", "USD", "FIFO", "wallet");
    case.tx("cheap", "w", "buy", "2025-01-01T12:00:00Z", BTC, "100");
    case.tx("dear", "b", "buy", "2025-05-01T12:00:00Z", BTC, "900");
    case.process();
    let summary = case.summary(dict([
        ("year", 2025.into()),
        ("wallets", Value::List(vec![s("w")])),
    ]));
    let row = &list(at(&summary, &["wallet_holdings"]))[0];
    assert_eq!(number(at(row, &["cost_basis"])), 100.);
}

#[test]
fn the_review_rounds_an_exact_half_cent_as_the_austrian_form_does() {
    let case = Case::new("at", "EUR", "MOVING_AVERAGE_AT", "global");
    case.tx("buy", "w", "buy", "2023-01-02T12:00:00Z", BTC, "100");
    case.tx(
        "sell",
        "w",
        "sell",
        "2024-01-02T12:00:00Z",
        BTC,
        "101.00499999999999999",
    );
    case.process();
    let (codes, _) =
        review_figures::kennzahl_summary(case.book.connection(), &case.profile().1, 2024).unwrap();
    assert_eq!(codes.iter().find(|(n, _)| *n == 174).unwrap().1, 100.into());
    let cut = tax_years::Cut::of(case.book.connection(), &case.profile().1).unwrap();
    let totals = review_figures::year_totals(case.book.connection(), "p", cut).unwrap();
    assert_eq!(
        kassiber_transfers::review_delta::money(&totals[&2024][0]).unwrap(),
        "1.00"
    );
}

#[test]
fn a_swiss_year_without_trades_still_states_its_wealth() {
    let case = Case::new("ch", "CHF", "FIFO", "global");
    case.tx("buy", "w", "buy", "2023-05-01T12:00:00Z", BTC, "100");
    case.tx("sell", "w", "sell", "2025-07-10T12:00:00Z", BTC, "400");
    case.sql(
        "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at)
         VALUES ('BTC-CHF','2024-12-31T12:00:00Z',300,'300','estv_kursliste','2025')",
    );
    case.process();
    let model = ch_tax::model(case.book.connection(), &case.profile().1, None).unwrap();
    let years = list(at(&model, &["years"]));
    let labels: Vec<f64> = years.iter().map(|y| number(at(y, &["year"]))).collect();
    assert_eq!(labels, [2023., 2024., 2025.]);
    assert_eq!(number(at(&years[1], &["wealth_total"])), 300.);
}

#[test]
fn a_swiss_coin_held_since_its_last_trade_is_stated_at_the_latest_value() {
    let case = Case::new("ch", "CHF", "FIFO", "global");
    case.tx("buy", "w", "buy", "2023-05-01T12:00:00Z", BTC, "100");
    case.sql(
        "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at)
         VALUES ('BTC-CHF','2025-12-31T12:00:00Z',500,'500','estv_kursliste','2026')",
    );
    case.process();
    let model = ch_tax::model(case.book.connection(), &case.profile().1, None).unwrap();
    let years = list(at(&model, &["years"]));
    let labels: Vec<f64> = years.iter().map(|y| number(at(y, &["year"]))).collect();
    assert_eq!(labels, [2023., 2024., 2025.]);
    assert_eq!(number(at(&years[2], &["wealth_total"])), 500.);
}

#[test]
fn swiss_purchase_volume_reads_the_booked_value_of_a_rate_priced_buy() {
    let case = Case::new("ch", "CHF", "FIFO", "global");
    case.tx("buy", "w", "buy", "2024-05-01T12:00:00Z", 2 * BTC, "100");
    case.process();
    let loaded = ch_tax::load(case.book.connection(), "p", "Europe/Zurich", None).unwrap();
    assert_eq!(loaded.purchases[0].1, "200".parse().unwrap());
}

#[test]
fn a_uk_sale_reviewed_as_not_taxable_leaves_the_pool_but_not_sa108() {
    let case = Case::new("gb", "GBP", "UK_SHARE_MATCHING", "global");
    case.tx("buy", "w", "buy", "2024-05-01T12:00:00Z", 2 * BTC, "100");
    case.tx("out", "w", "sell", "2024-07-10T12:00:00Z", BTC, "10000");
    case.tx("sale", "w", "sell", "2024-08-10T12:00:00Z", BTC, "400");
    case.sql("UPDATE transactions SET taxability_override=0 WHERE id='out'");
    case.process();
    let model = gb_tax::model(case.book.connection(), &case.profile().1, None).unwrap();
    let year = &list(at(&model, &["years"]))[0];
    // Only the later sale counts, at the pool's 100 for the coin left.
    assert_eq!(number(at(year, &["net_gains"])), 300.);
    assert_eq!(number(at(year, &["disposals"])), 1.);
    assert_eq!(list(at(&model, &["computations"])).len(), 1);
    assert_eq!(case.review(2024), 300.);
}

#[test]
fn a_uk_day_mixing_taxable_and_non_taxable_sales_is_refused() {
    let case = Case::new("gb", "GBP", "UK_SHARE_MATCHING", "global");
    case.tx("buy", "w", "buy", "2024-05-01T12:00:00Z", 2 * BTC, "100");
    case.tx("out", "w", "sell", "2024-07-10T10:00:00Z", BTC, "400");
    case.tx("sale", "w", "sell", "2024-07-10T12:00:00Z", BTC, "400");
    case.sql("UPDATE transactions SET taxability_override=0 WHERE id='out'");
    case.process();
    let model = gb_tax::model(case.book.connection(), &case.profile().1, None).unwrap();
    let refusals = list(at(&model, &["refusals"]));
    assert_eq!(refusals.len(), 1);
    assert_eq!(
        at(&refusals[0], &["code"]),
        &s("non_taxable_same_day_disposal")
    );
}

#[test]
fn a_japanese_year_with_a_non_taxable_sale_is_withheld_and_its_coins_carry() {
    let case = Case::new("jp", "JPY", "JP_TOTAL_AVERAGE", "global");
    case.tx("buy", "w", "buy", "2024-05-01T12:00:00Z", 2 * BTC, "100");
    case.tx("out", "w", "sell", "2024-07-10T12:00:00Z", BTC, "10000");
    case.tx("next", "w", "buy", "2025-05-01T12:00:00Z", BTC, "100");
    case.sql("UPDATE transactions SET taxability_override=0 WHERE id='out'");
    case.process();
    let model = jp_tax::model(case.book.connection(), &case.profile().1, None).unwrap();
    let years = list(at(&model, &["years"]));
    let year = |y: f64| {
        years
            .iter()
            .find(|v| number(at(v, &["year"])) == y)
            .unwrap()
    };
    assert_eq!(at(year(2024.), &["return_income"]), &Value::Null);
    let next = &list(at(year(2025.), &["sheets"]))[0];
    assert_eq!(number(at(next, &["opening_qty"])), 1.);
}
