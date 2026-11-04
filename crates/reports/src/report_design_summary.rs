//! The summary report in the shared design, from the summary model
//! (`build_summary_pdf_report_data`), the book's tax settings and, for
//! Austria, the E 1kv Kennzahl totals of each tax year in the period.
use super::*;
use crate::report_design::{
    display_time, from_float, msat_of, Block, Cell, Chart, Column, Document, Fact, Kind, Kpi,
    Legend, Section, Series, Table,
};

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    artifact_csv::list(value(v, key))
}
fn fiat(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Null => Cell::Empty,
        raw => Cell::Fiat(from_float(raw)),
    }
}
fn btc(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Null => Cell::Empty,
        raw => Cell::Btc(from_float(raw)),
    }
}
fn msat(cell: &Cell) -> Cell {
    match cell {
        Cell::Btc(d) => msat_of(d),
        _ => Cell::Empty,
    }
}
fn txt(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Null => Cell::Empty,
        raw => Cell::Text(py_str(raw)),
    }
}
fn day(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Null => Cell::Empty,
        raw => Cell::Date(py_str(raw).chars().take(10).collect()),
    }
}
fn count(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Int(n) => Cell::Count(i64::try_from(n.clone()).unwrap_or(0)),
        _ => Cell::Empty,
    }
}
fn col(key: &'static str, label: impl Into<String>, kind: Kind, width: f64) -> Column {
    Column {
        key,
        label: label.into(),
        kind,
        width,
    }
}
fn fact(key: &'static str, label: &str, value: Cell) -> Fact {
    Fact {
        key,
        label: label.into(),
        value,
        display: None,
        accent: false,
    }
}
fn fiat_label(label: &str, currency: &str) -> String {
    format!("{label} ({currency})")
}
fn method_name(algorithm: &str) -> String {
    match algorithm {
        "FIFO" => "First in, first out (FIFO)".into(),
        "LIFO" => "Last in, first out (LIFO)".into(),
        "HIFO" => "Highest in, first out (HIFO)".into(),
        "LOFO" => "Lowest in, first out (LOFO)".into(),
        "MOVING_AVERAGE" => "Moving average".into(),
        "MOVING_AVERAGE_AT" => "Moving average (Austrian rules)".into(),
        other => other.into(),
    }
}

/// The E 1kv form wording of each Kennzahl the report fills, as Kassiber's
/// Austrian PDF prints it for FinanzOnline.
const KENNZAHLEN: [(&str, &str); 4] = [
    ("172", "Ausländische laufende Einkünfte"),
    (
        "174",
        "Ausländische Überschüsse aus realisierten Wertsteigerungen",
    ),
    ("176", "Ausländische Verluste"),
    ("801", "Einkünfte aus Spekulationsgeschäften (§ 31)"),
];

fn ticks(low: f64, high: f64, count: usize, format: impl Fn(f64) -> String) -> Vec<(f64, String)> {
    (0..count)
        .map(|i| {
            let at = i as f64 / (count - 1) as f64;
            (at, format(low + (high - low) * at))
        })
        .collect()
}
fn compact(value: f64) -> String {
    let magnitude = value.abs();
    let sign = if value < 0.0 { "\u{2212}" } else { "" };
    if magnitude >= 1_000_000.0 {
        format!("{sign}{:.1}m", magnitude / 1_000_000.0)
    } else if magnitude >= 1_000.0 {
        format!("{sign}{:.0}k", magnitude / 1_000.0)
    } else {
        format!("{sign}{magnitude:.0}")
    }
}
fn bounds(values: &[f64]) -> (f64, f64) {
    let low = values
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min)
        .min(0.0);
    let high = values
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        .max(0.0);
    if high == low {
        (low, low + 1.0)
    } else {
        (low, high + (high - low) * 0.05)
    }
}
fn unit(v: f64, low: f64, high: f64) -> f64 {
    (v - low) / (high - low)
}
fn month_labels(months: &[String]) -> Vec<(f64, String)> {
    let total = months.len();
    let step = total.div_ceil(8).max(1);
    months
        .iter()
        .enumerate()
        .filter(|(i, _)| i % step == 0 || *i + 1 == total)
        .map(|(i, m)| ((i as f64 + 0.5) / total as f64, m.clone()))
        .collect()
}

fn balance_chart(history: &[Value], currency: &str) -> Option<Chart> {
    if history.is_empty() {
        return None;
    }
    let f = |row: &Value, key: &str| -> f64 {
        from_float(value(row, key))
            .to_canonical()
            .parse()
            .unwrap_or(0.0)
    };
    let market: Vec<f64> = history.iter().map(|r| f(r, "market_value")).collect();
    let basis: Vec<f64> = history
        .iter()
        .map(|r| f(r, "cumulative_cost_basis"))
        .collect();
    let quantity: Vec<f64> = history.iter().map(|r| f(r, "quantity")).collect();
    let (low, high) = bounds(&[market.clone(), basis.clone()].concat());
    let (qlow, qhigh) = bounds(&quantity);
    let total = history.len();
    let x = |i: usize| (i as f64 + 0.5) / total as f64;
    let line = |values: &[f64], lo: f64, hi: f64| {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| (x(i), unit(*v, lo, hi)))
            .collect()
    };
    let months: Vec<String> = history.iter().map(|r| text(r, "period")).collect();
    Some(Chart {
        title: "Balance at month end".into(),
        height: 120.0,
        y_labels: ticks(low, high, 4, compact),
        y2_labels: ticks(qlow, qhigh, 4, |v| format!("{v:.2}")),
        x_labels: month_labels(&months),
        series: vec![
            Series::Line {
                points: line(&quantity, qlow, qhigh),
                weight: 0.6,
                dashed: false,
                tone: "muted",
            },
            Series::Line {
                points: line(&basis, low, high),
                weight: 0.8,
                dashed: true,
                tone: "ink",
            },
            Series::Line {
                points: line(&market, low, high),
                weight: 1.4,
                dashed: false,
                tone: "accent",
            },
        ],
        legend: vec![
            Legend {
                label: format!("Market value ({currency}, left axis)"),
                mark: "line",
                tone: "accent",
            },
            Legend {
                label: format!("Cost basis ({currency}, left axis)"),
                mark: "dashed",
                tone: "ink",
            },
            Legend {
                label: "Quantity (BTC, right axis)".into(),
                mark: "thin",
                tone: "muted",
            },
        ],
    })
}

fn flow_chart(flows: &[Value], currency: &str) -> Option<Chart> {
    if flows.is_empty() {
        return None;
    }
    let f = |row: &Value, key: &str| -> f64 {
        from_float(value(row, key))
            .to_canonical()
            .parse()
            .unwrap_or(0.0)
    };
    let inflow: Vec<f64> = flows.iter().map(|r| f(r, "inflow_volume")).collect();
    let outflow: Vec<f64> = flows.iter().map(|r| -f(r, "outflow_volume")).collect();
    let (low, high) = bounds(&[inflow.clone(), outflow.clone()].concat());
    let total = flows.len() as f64;
    let zero = unit(0.0, low, high);
    let bars = |values: &[f64]| {
        values
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0.0)
            .map(|(i, v)| {
                (
                    i as f64 / total + 0.15 / total,
                    (i as f64 + 0.85) / total,
                    zero,
                    unit(*v, low, high),
                )
            })
            .collect()
    };
    let months: Vec<String> = flows.iter().map(|r| text(r, "period")).collect();
    Some(Chart {
        title: "Monthly flows".into(),
        height: 90.0,
        y_labels: ticks(low, high, 4, compact),
        y2_labels: vec![],
        x_labels: month_labels(&months),
        series: vec![
            Series::Bars {
                bars: bars(&inflow),
                hollow: false,
                tone: "ink",
            },
            Series::Bars {
                bars: bars(&outflow),
                hollow: true,
                tone: "ink",
            },
        ],
        legend: vec![
            Legend {
                label: format!("Inflow ({currency}, up)"),
                mark: "filled",
                tone: "ink",
            },
            Legend {
                label: format!("Outflow ({currency}, down)"),
                mark: "hollow",
                tone: "ink",
            },
        ],
    })
}

/// The summary report document. `model` holds `summary` (Python's summary
/// model), `profile` (tax settings) and `kennzahlen` (Austria only).
pub fn summary(model: &Value) -> Document {
    let report = value(model, "summary");
    let profile = value(model, "profile");
    let currency = text(report, "fiat_currency");
    let country = text(profile, "tax_country");
    let austria = country == "at";
    let timeframe = value(report, "timeframe");
    let integrity = value(report, "data_integrity");
    let journals = value(integrity, "journals");
    let metrics = value(report, "metrics");
    let input_version = int(journals, "journal_input_version");
    let mut sections = Vec::new();

    // 1. Scope
    let wallets: Vec<String> = list(report, "wallets")
        .iter()
        .map(|w| text(w, "label"))
        .collect();
    let status = text(journals, "status");
    let mut scope = vec![
        fact(
            "book",
            "Book",
            Cell::Text(format!(
                "{} / {}",
                text(report, "workspace"),
                text(report, "profile")
            )),
        ),
        fact("period_start", "Period start", day(timeframe, "start")),
        fact("period_end", "Period end", day(timeframe, "end")),
        Fact {
            display: Some(wallets.join(", ")),
            ..fact("wallets", "Wallets", Cell::Count(wallets.len() as i64))
        },
        fact(
            "fiat_currency",
            "Fiat currency",
            Cell::Text(currency.clone()),
        ),
        Fact {
            display: Some(if austria {
                "Austria".into()
            } else {
                "Generic".into()
            }),
            ..fact("tax_country", "Tax country", Cell::Text(country.clone()))
        },
        fact(
            "method",
            "Cost-basis method",
            Cell::Text(method_name(&text(profile, "gains_algorithm"))),
        ),
        Fact {
            display: Some(display_time(&text(report, "generated_at"))),
            ..fact(
                "generated_at",
                "Generated",
                Cell::Text(text(report, "generated_at")),
            )
        },
        fact(
            "journal_input_version",
            "Journal input version",
            Cell::Count(input_version),
        ),
    ];
    scope.push(Fact {
        accent: status != "current",
        ..fact("journals", "Journals", Cell::Text(status.replace('_', " ")))
    });
    sections.push(Section {
        id: "scope",
        title: "Scope".into(),
        lead: String::new(),
        page_break: false,
        blocks: vec![Block::Facts(scope)],
    });

    // 2. Summary
    let start = from_float(value(metrics, "period_start_value"));
    let end = from_float(value(metrics, "period_end_value"));
    let change = if start > Decimal::zero() {
        let pct = end
            .sub(&start, &DECIMAL)
            .and_then(|d| d.div(&start, &DECIMAL))
            .and_then(|d| d.mul(&Decimal::from(100), &DECIMAL))
            .ok();
        pct.map(|p| format!("{} vs start", Cell::Percent(p).display()))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let benchmark = value(report, "benchmark");
    let spot = match value(benchmark, "change_pct") {
        Value::Null => String::new(),
        raw => format!("BTC spot {}", Cell::Percent(from_float(raw)).display()),
    };
    let age = value(report, "holding_age");
    let age_detail = match value(age, "weighted_days") {
        Value::Null => String::new(),
        raw => format!(
            "weighted age {} days",
            Cell::Count(
                from_float(raw)
                    .to_fixed_places(0, Rounding::HalfEven)
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(0)
            )
            .display()
        ),
    };
    let kpis = vec![
        Kpi {
            key: "start_value",
            label: fiat_label("Value at start", &currency),
            value: fiat(metrics, "period_start_value"),
            detail: String::new(),
            accent: false,
        },
        Kpi {
            key: "end_value",
            label: fiat_label("Value at end", &currency),
            value: fiat(metrics, "period_end_value"),
            detail: [change, spot]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
            accent: false,
        },
        Kpi {
            key: "net_flow",
            label: fiat_label("Net inflow", &currency),
            value: fiat(metrics, "net_flow"),
            detail: String::new(),
            accent: false,
        },
        Kpi {
            key: "realized_gain",
            label: fiat_label("Realized gain", &currency),
            value: fiat(metrics, "realized_pnl"),
            detail: String::new(),
            accent: false,
        },
        Kpi {
            key: "unrealized_gain",
            label: fiat_label("Unrealized gain at end", &currency),
            value: fiat(metrics, "unrealized_pnl"),
            detail: String::new(),
            accent: false,
        },
        Kpi {
            key: "cost_basis_end",
            label: fiat_label("Cost basis at end", &currency),
            value: fiat(metrics, "end_cost_basis"),
            detail: String::new(),
            accent: false,
        },
        Kpi {
            key: "quantity_end",
            label: "Quantity at end (BTC)".into(),
            value: btc(metrics, "btc_stack_end"),
            detail: format!(
                "start {}{}",
                btc(metrics, "btc_stack_start").display(),
                if age_detail.is_empty() {
                    String::new()
                } else {
                    format!(" · {age_detail}")
                }
            ),
            accent: false,
        },
        Kpi {
            key: "fees",
            label: fiat_label("Fees", &currency),
            value: fiat(metrics, "fees_fiat"),
            detail: format!("{} BTC", btc(metrics, "fees_btc").display()),
            accent: false,
        },
    ];
    sections.push(Section {
        id: "summary",
        title: "Summary".into(),
        lead: String::new(),
        page_break: false,
        blocks: vec![Block::Kpis(kpis)],
    });

    // 3. Holdings at period end (and now, when the snapshot was asked for)
    let holdings_table =
        |id: &'static str, title: String, rows: &[Value], totals: &Value| -> Table {
            let columns = vec![
                col("wallet", "Wallet", Kind::Text, 1.4),
                col("scope", "Scope", Kind::Text, 1.2),
                col("assets", "Assets", Kind::Text, 0.8),
                col("quantity_btc", "Quantity (BTC)", Kind::Btc, 1.3),
                col("quantity_msat", "Quantity (msat)", Kind::Msat, 1.0),
                col(
                    "cost_basis",
                    fiat_label("Cost basis", &currency),
                    Kind::Fiat,
                    1.2,
                ),
                col(
                    "market_value",
                    fiat_label("Market value", &currency),
                    Kind::Fiat,
                    1.2,
                ),
                col("transactions", "Transactions", Kind::Count, 0.9),
            ];
            let rows = rows
                .iter()
                .map(|row| {
                    let quantity = btc(row, "quantity");
                    vec![
                        txt(row, "wallet"),
                        txt(row, "scope"),
                        Cell::Text(
                            list(row, "assets")
                                .iter()
                                .map(py_str)
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                        quantity.clone(),
                        msat(&quantity),
                        fiat(row, "cost_basis"),
                        fiat(row, "market_value"),
                        count(row, "tx_count"),
                    ]
                })
                .collect::<Vec<_>>();
            let total_quantity = btc(totals, "total_quantity");
            let blank = || Cell::Text(String::new());
            let total = vec![
                Cell::Text("Total".into()),
                blank(),
                blank(),
                total_quantity.clone(),
                msat(&total_quantity),
                blank(),
                fiat(totals, "total_market_value"),
                blank(),
            ];
            Table {
                id,
                title,
                columns,
                rows,
                total: Some(total),
                note: String::new(),
            }
        };
    let mut holdings = vec![Block::Table(holdings_table(
        "holdings_period_end",
        format!(
            "At the end of the period ({})",
            text(timeframe, "end").chars().take(10).collect::<String>()
        ),
        list(report, "wallet_holdings"),
        value(report, "holdings_totals"),
    ))];
    let snapshot = value(report, "snapshot");
    if truthy(snapshot) {
        let mut table = holdings_table(
            "holdings_now",
            format!("Now ({})", display_time(&text(snapshot, "as_of"))),
            list(snapshot, "wallets"),
            snapshot,
        );
        table.note = "Valued at the latest rate when the report was generated.".into();
        holdings.push(Block::Table(table));
    }
    sections.push(Section {
        id: "holdings",
        title: "Holdings".into(),
        lead: "Per wallet at the end of the period. Wallet cost basis is an allocation at the asset's average basis, not a claim about which lots a wallet holds.".into(),
        page_break: false,
        blocks: holdings,
    });

    // 4. Activity: charts and the month table
    let history = list(report, "balance_history");
    let flows = list(report, "flow_periods");
    let realized = list(report, "realized_pnl_periods");
    let mut blocks = Vec::new();
    if let Some(chart) = balance_chart(history, &currency) {
        blocks.push(Block::Chart(chart));
    }
    if let Some(chart) = flow_chart(flows, &currency) {
        blocks.push(Block::Chart(chart));
    }
    let by_month =
        |rows: &[Value], period: &str| rows.iter().find(|r| text(r, "period") == period).cloned();
    let months = vec![
        col("month", "Month", Kind::Date, 0.8),
        col("inflow", fiat_label("Inflow", &currency), Kind::Fiat, 1.1),
        col("outflow", fiat_label("Outflow", &currency), Kind::Fiat, 1.1),
        col(
            "realized_gain",
            fiat_label("Realized gain", &currency),
            Kind::Fiat,
            1.1,
        ),
        col("disposals", "Disposals", Kind::Count, 0.7),
        col("fees", fiat_label("Fees", &currency), Kind::Fiat, 0.8),
        col("fees_btc", "Fees (BTC)", Kind::Btc, 1.0),
        col("fees_msat", "Fees (msat)", Kind::Msat, 1.0),
        col("quantity_btc", "Quantity at end (BTC)", Kind::Btc, 1.2),
        col("quantity_msat", "Quantity at end (msat)", Kind::Msat, 1.0),
        col(
            "market_value",
            fiat_label("Value at end", &currency),
            Kind::Fiat,
            1.1,
        ),
    ];
    let mut rows = Vec::new();
    for flow in flows {
        let period = text(flow, "period");
        let gain = by_month(realized, &period).unwrap_or(Value::Null);
        let balance = by_month(history, &period).unwrap_or(Value::Null);
        let fees = btc(flow, "fees_btc");
        let quantity = btc(&balance, "quantity");
        rows.push(vec![
            Cell::Date(period),
            fiat(flow, "inflow_volume"),
            fiat(flow, "outflow_volume"),
            fiat(&gain, "realized_pnl"),
            count(&gain, "count"),
            fiat(flow, "fees_fiat"),
            fees.clone(),
            msat(&fees),
            quantity.clone(),
            msat(&quantity),
            fiat(&balance, "market_value"),
        ]);
    }
    let partial = history
        .last()
        .is_some_and(|r| truthy(value(r, "period_partial")));
    blocks.push(Block::Table(Table {
        id: "monthly_activity",
        title: "By month".into(),
        columns: months,
        rows,
        total: None,
        note: if partial {
            format!(
                "The last month ends with the period on {}.",
                text(timeframe, "end").chars().take(10).collect::<String>()
            )
        } else {
            String::new()
        },
    }));
    sections.push(Section {
        id: "activity",
        title: "Activity".into(),
        lead: "Internal transfers between the book's own wallets are not counted as flows.".into(),
        page_break: true,
        blocks,
    });

    // 5. Largest disposals and movements
    let disposals = list(report, "top_disposals");
    let movements = list(report, "top_movements");
    let mut largest = Vec::new();
    largest.push(Block::Table(Table {
        id: "largest_disposals",
        title: "Largest disposals".into(),
        columns: vec![
            col("date", "Date", Kind::Date, 0.9),
            col("wallet", "Wallet", Kind::Text, 1.1),
            col("asset", "Asset", Kind::Text, 0.6),
            col("quantity_btc", "Quantity (BTC)", Kind::Btc, 1.2),
            col("quantity_msat", "Quantity (msat)", Kind::Msat, 1.0),
            col(
                "proceeds",
                fiat_label("Proceeds", &currency),
                Kind::Fiat,
                1.1,
            ),
            col(
                "cost_basis",
                fiat_label("Cost basis", &currency),
                Kind::Fiat,
                1.1,
            ),
            col("gain", fiat_label("Gain", &currency), Kind::Fiat, 1.1),
        ],
        rows: disposals
            .iter()
            .map(|row| {
                // The model keeps the journal's signed disposal quantity.
                let quantity = match btc(row, "quantity") {
                    Cell::Btc(d) => Cell::Btc(d.neg(&DECIMAL).unwrap_or(d)),
                    other => other,
                };
                vec![
                    day(row, "occurred_at"),
                    txt(row, "wallet"),
                    txt(row, "asset"),
                    quantity.clone(),
                    msat(&quantity),
                    fiat(row, "proceeds"),
                    fiat(row, "cost_basis"),
                    fiat(row, "gain_loss"),
                ]
            })
            .collect(),
        total: None,
        note: if disposals.is_empty() {
            "No disposals in the period.".into()
        } else {
            "The five disposals with the largest gain or loss.".into()
        },
    }));
    largest.push(Block::Table(Table {
        id: "largest_movements",
        title: "Largest movements".into(),
        columns: vec![
            col("date", "Date", Kind::Date, 0.9),
            col("wallet", "Wallet", Kind::Text, 1.0),
            col("direction", "Direction", Kind::Text, 0.8),
            col("asset", "Asset", Kind::Text, 0.6),
            col("quantity_btc", "Quantity (BTC)", Kind::Btc, 1.2),
            col("quantity_msat", "Quantity (msat)", Kind::Msat, 1.0),
            col("value", fiat_label("Value", &currency), Kind::Fiat, 1.1),
            col(
                "counterparty",
                "Counterparty or description",
                Kind::Text,
                1.6,
            ),
        ],
        rows: movements
            .iter()
            .map(|row| {
                let quantity = btc(row, "quantity");
                vec![
                    day(row, "occurred_at"),
                    txt(row, "wallet"),
                    txt(row, "direction"),
                    txt(row, "asset"),
                    quantity.clone(),
                    msat(&quantity),
                    fiat(row, "fiat_value"),
                    txt(row, "counterparty"),
                ]
            })
            .collect(),
        total: None,
        note: if movements.is_empty() {
            "No transactions in the period.".into()
        } else {
            "The five transactions with the largest value.".into()
        },
    }));
    sections.push(Section {
        id: "largest",
        title: "Largest disposals and movements".into(),
        lead: String::new(),
        page_break: false,
        blocks: largest,
    });

    // 6. Country layer: Austria's E 1kv Kennzahlen per tax year.
    if austria {
        let mut rows = Vec::new();
        if let Value::Dict(years) = value(model, "kennzahlen") {
            for (year, totals) in years {
                let year = match year {
                    kassiber_pyjson::Key::Str(y) => y.as_str().unwrap_or_default().to_owned(),
                    other => format!("{other:?}"),
                };
                for (code, wording) in KENNZAHLEN {
                    let row = value(totals, code);
                    let cents = match value(row, "amount_eur_cents") {
                        Value::Int(n) => n.clone(),
                        _ => 0.into(),
                    };
                    let amount = cents
                        .to_string()
                        .parse::<Decimal>()
                        .ok()
                        .and_then(|d| d.div(&Decimal::from(100), &DECIMAL).ok())
                        .unwrap_or_else(Decimal::zero);
                    rows.push(vec![
                        Cell::Text(year.clone()),
                        Cell::Text(code.into()),
                        Cell::Text(wording.into()),
                        Cell::Text(if code == "801" {
                            "E 1".into()
                        } else {
                            "E 1kv".into()
                        }),
                        Cell::Fiat(amount),
                        count(row, "row_count"),
                    ]);
                }
            }
        }
        sections.push(Section {
            id: "country_at",
            title: "Austria: E 1kv Kennzahlen".into(),
            lead: "Totals per tax year (Europe/Vienna calendar) as filed in FinanzOnline. Kassiber maps self-custody crypto to the foreign (ausländisch) Kennzahlen.".into(),
            page_break: false,
            blocks: vec![
                Block::Table(Table {
                    id: "e1kv_kennzahlen",
                    title: String::new(),
                    columns: vec![
                        col("tax_year", "Tax year", Kind::Code, 0.6),
                        col("kennzahl", "KZ", Kind::Code, 0.4),
                        col("field", "Field", Kind::Text, 3.2),
                        col("form", "Form", Kind::Text, 0.6),
                        col("amount_eur", "Amount (EUR)", Kind::Fiat, 1.0),
                        col("rows", "Rows", Kind::Count, 0.5),
                    ],
                    rows,
                    total: None,
                    note: String::new(),
                }),
                Block::Note {
                    text: "Domestic-provider Kennzahlen 171, 173 and 175 (withheld KESt) are not modelled and stay empty; check them against the provider's own report.".into(),
                    accent: false,
                },
            ],
        });
    }

    // 7. Open issues
    let total_tx = int(integrity, "total_transactions");
    let priced = int(integrity, "priced_transactions");
    let internal = value(integrity, "internal_transfers");
    let mut issues = vec![
        Fact {
            display: Some(format!(
                "{} of {} ({})",
                Cell::Count(priced).display(),
                Cell::Count(total_tx).display(),
                Cell::Percent(from_float(value(integrity, "priced_percentage"))).display()
            )),
            accent: priced < total_tx,
            ..fact(
                "priced_transactions",
                "Priced transactions",
                Cell::Count(priced),
            )
        },
        Fact {
            accent: int(integrity, "quarantine_count") > 0,
            ..fact(
                "quarantines",
                "Quarantined transactions",
                count(integrity, "quarantine_count"),
            )
        },
        Fact {
            display: Some(format!(
                "{} transfers · {} {}",
                Cell::Count(int(internal, "count")).display(),
                fiat(internal, "fiat_volume").display(),
                currency
            )),
            ..fact(
                "internal_transfers",
                "Internal transfers (not flows)",
                count(internal, "count"),
            )
        },
    ];
    issues.push(Fact {
        accent: status != "current",
        ..fact(
            "journal_status",
            "Journal status",
            Cell::Text(status.replace('_', " ")),
        )
    });
    let mut blocks = vec![Block::Facts(issues)];
    let reasons = list(integrity, "quarantine_reasons");
    if reasons.is_empty() {
        blocks.push(Block::Note {
            text: "No quarantined transactions in the period.".into(),
            accent: false,
        });
    } else {
        blocks.push(Block::Table(Table {
            id: "quarantines",
            title: "Quarantine reasons".into(),
            columns: vec![
                col("reason", "Reason", Kind::Code, 3.0),
                col("transactions", "Transactions", Kind::Count, 1.0),
            ],
            rows: reasons
                .iter()
                .map(|r| vec![txt(r, "reason"), count(r, "count")])
                .collect(),
            total: None,
            note: "Quarantined transactions stay out of the figures until they are resolved."
                .into(),
        }));
    }
    sections.push(Section {
        id: "issues",
        title: "Open issues".into(),
        lead: String::new(),
        page_break: false,
        blocks,
    });

    // 8. Method and provenance
    let mut method = vec![
        fact("method", "Cost-basis method", Cell::Text(method_name(&text(profile, "gains_algorithm")))),
        fact("engine", "Engine", Cell::Text("Kassiber tax engine over the processed journal".into())),
        fact("valuation", "Valuation", Cell::Text(format!("BTC-{currency} rate cache at or before each date; transaction prices where the cache has none"))),
    ];
    if austria {
        method.push(fact("regime", "Regimes", Cell::Text("Acquisitions before 2021-03-01 are Altvermögen (one-year speculation period); later ones Neuvermögen".into())));
    } else {
        let days = int(profile, "tax_long_term_days");
        method.push(fact(
            "long_term_days",
            "Long-term threshold (days)",
            Cell::Count(days),
        ));
    }
    let mut blocks = vec![Block::Facts(method)];
    blocks.push(Block::Note {
        text: if austria {
            "Review this export with a Steuerberater before filing; Kassiber is not tax advice."
                .into()
        } else {
            "A portfolio and treasury view of the processed journal; it is not a tax filing.".into()
        },
        accent: false,
    });
    sections.push(Section {
        id: "method",
        title: "Method and provenance".into(),
        lead: String::new(),
        page_break: false,
        blocks,
    });

    let period = format!(
        "{} to {}",
        text(timeframe, "start")
            .chars()
            .take(10)
            .collect::<String>(),
        text(timeframe, "end").chars().take(10).collect::<String>()
    );
    Document {
        kind: "summary",
        title: "Summary report".into(),
        kicker: if austria {
            "Kassiber · Austria".into()
        } else {
            "Kassiber".into()
        },
        book: format!(
            "{} / {}",
            text(report, "workspace"),
            text(report, "profile")
        ),
        period,
        period_start: text(timeframe, "start"),
        period_end: text(timeframe, "end"),
        generated_at: text(report, "generated_at"),
        input_version,
        fiat: currency,
        country,
        sections,
    }
}

/// The data the summary document is built from, read like Python's
/// reports: the summary model, the profile's tax settings and, for
/// Austria, the E 1kv Kennzahl totals of each tax year the period covers.
pub fn summary_model(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    require_context(conn, &profile, false)?;
    let report = summary_pdf::model(conn, &workspace, &profile, args, sources)?;
    let settings = dict(
        [
            "fiat_currency",
            "gains_algorithm",
            "journal_input_version",
            "tax_country",
            "tax_long_term_days",
        ]
        .map(|key| (key, value(&profile, key).clone())),
    );
    let mut kennzahlen = Vec::new();
    if text(&profile, "tax_country") == "at" {
        let timeframe = value(&report, "timeframe");
        let first: i64 = text(timeframe, "start")[..4].parse().unwrap_or(0);
        let last: i64 = text(timeframe, "end")[..4].parse().unwrap_or(0);
        for year in first..=last {
            let annual = austrian::report(
                conn,
                &workspace,
                &profile,
                &dict([("year", Value::Int(year.into()))]),
            )?;
            kennzahlen.push((year.to_string(), value(&annual, "kennzahl_totals").clone()));
        }
    }
    Ok(dict([
        ("summary", report),
        ("profile", settings),
        ("kennzahlen", dict(kennzahlen)),
    ]))
}
