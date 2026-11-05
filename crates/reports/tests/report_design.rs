//! The shared report design's summary prototype: the data handed to the
//! renderers equals Python's summary model, and the PDF, XLSX and CSV bundle
//! are deterministic and machine-clean.
use kassiber_pyjson::Value;
use kassiber_reports::report_design;
use kassiber_store::{rusqlite::Connection, Book};

struct Fixed;
impl kassiber_setup::Sources for Fixed {
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-01-02T03:04:05Z".into())
    }
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        panic!("rendering minted an identity")
    }
}

fn encode(v: &Value) -> String {
    kassiber_pyjson::dumps(v, &Default::default()).unwrap()
}
fn get<'a>(v: &'a Value, key: &str) -> &'a Value {
    kassiber_envelope::get(v, key).unwrap_or(&Value::Null)
}

fn check_csv(files: &[(String, Vec<u8>)], name: &str) {
    let manifest: serde_json::Value =
        serde_json::from_slice(&files.last().unwrap().1).expect("manifest.json");
    assert_eq!(files.last().unwrap().0, "manifest.json");
    assert_eq!(manifest["schema"], "kassiber.report/1");
    let listed = manifest["files"].as_array().unwrap();
    assert_eq!(listed.len(), files.len() - 1, "{name}");
    for ((file, bytes), entry) in files.iter().zip(listed) {
        assert_eq!(entry["name"], file.as_str());
        let digest = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes));
        assert_eq!(entry["sha256"], digest.as_str(), "{name}: {file}");
        let text = String::from_utf8(bytes.clone()).expect("UTF-8");
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().unwrap().split(',').collect();
        for column in &header {
            assert!(
                column
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{name}: {file}: header {column}"
            );
        }
        assert_eq!(
            entry["rows"].as_u64().unwrap() as usize,
            lines.clone().count()
        );
        for line in lines {
            for field in line.split(',') {
                // Plain decimals: no exponent, no grouping, ASCII minus.
                if field
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '-')
                {
                    assert!(!field.contains(['e', 'E', ',']), "{name}: {file}: {field}");
                }
            }
        }
        for (i, column) in header.iter().enumerate() {
            if column.ends_with("_msat") {
                for line in text.lines().skip(1) {
                    let field = line.split(',').nth(i).unwrap_or("");
                    assert!(
                        field.is_empty() || field.parse::<i64>().is_ok(),
                        "{name}: {file}: {field}"
                    );
                }
            }
        }
    }
}

#[test]
fn summary_model_matches_python_and_renders_deterministically() {
    let cases = kassiber_pytext::loads(include_str!("data/summary_model.json")).unwrap();
    let Value::List(cases) = cases else { panic!() };
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    let mut rendered = 0;
    for case in &cases {
        let name = kassiber_envelope::py_str(get(case, "name"));
        conn.execute_batch("SAVEPOINT oracle_fixture").unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        conn.execute_batch(&kassiber_envelope::py_str(get(case, "sql")))
            .unwrap();
        let args =
            kassiber_pytext::loads(&kassiber_envelope::py_str(get(case, "args_json"))).unwrap();
        let expected = get(case, "expected");
        match report_design::summary_model(conn, &args, &Fixed) {
            Ok(model) => {
                let ok = get(expected, "ok");
                assert!(
                    !matches!(ok, Value::Null),
                    "{name}: Python failed with {}",
                    encode(expected)
                );
                assert_eq!(
                    encode(get(&model, "summary")),
                    encode(ok),
                    "{name}: summary model"
                );
                let document = report_design::summary(&model);
                let (pdf, pages) = report_design::pdf(&document).unwrap();
                assert!(pages >= 1 && pdf.starts_with(b"%PDF-"), "{name}");
                assert_eq!(
                    report_design::pdf(&document).unwrap().0,
                    pdf,
                    "{name}: PDF bytes"
                );
                let xlsx = report_design::xlsx(&document).unwrap();
                assert_eq!(
                    report_design::xlsx(&document).unwrap(),
                    xlsx,
                    "{name}: XLSX bytes"
                );
                let csv = report_design::csv(&document).unwrap();
                assert_eq!(
                    report_design::csv(&document).unwrap(),
                    csv,
                    "{name}: CSV bytes"
                );
                check_csv(&csv, &name);
                // The single-file bundle holds the same members, in order.
                let zip = report_design::csv_zip(&document).unwrap();
                assert!(zip.starts_with(b"PK\x03\x04"), "{name}: ZIP");
                assert_eq!(
                    report_design::csv_zip(&document).unwrap(),
                    zip,
                    "{name}: ZIP bytes"
                );
                let mut at = 0;
                for (member, _) in &csv {
                    let found = zip[at..]
                        .windows(member.len())
                        .position(|w| w == member.as_bytes())
                        .unwrap_or_else(|| panic!("{name}: {member} missing from the ZIP"));
                    at += found + member.len();
                }
                rendered += 1;
            }
            Err(e) => {
                let error = get(expected, "error");
                assert!(
                    !matches!(error, Value::Null),
                    "{name}: Rust failed with {e:?}"
                );
                assert_eq!(
                    e.code,
                    kassiber_envelope::py_str(get(error, "code")),
                    "{name}"
                );
                assert_eq!(
                    e.message,
                    kassiber_envelope::py_str(get(error, "message")),
                    "{name}"
                );
            }
        }
        conn.execute_batch("ROLLBACK TO oracle_fixture; RELEASE oracle_fixture")
            .unwrap();
    }
    assert!(rendered >= 15, "{rendered}");
}

#[test]
fn display_rounding_and_csv_text_come_from_one_exact_value() {
    use report_design::{Cell, Lang};
    let value = |raw: &str| raw.parse::<kassiber_pydecimal::Decimal>().unwrap();
    let (en, de) = (Lang::En, Lang::De);
    assert_eq!(
        Cell::Fiat(value("-1234.565")).display(en),
        "\u{2212}1,234.56"
    );
    assert_eq!(
        Cell::Fiat(value("-1234.565")).display(de),
        "\u{2212}1.234,56"
    );
    assert_eq!(Cell::Fiat(value("-1234.565")).csv(), "-1234.565");
    assert_eq!(Cell::Fiat(value("-0.001")).display(en), "0.00");
    assert_eq!(Cell::Btc(value("1.6473E-7")).display(de), "0,00000016");
    assert_eq!(Cell::Btc(value("1.6473E-7")).csv(), "0.00000016473");
    assert_eq!(Cell::Percent(value("59.25")).display(en), "59.2%");
    assert_eq!(Cell::Date("2025-03-15".into()).display(de), "15.03.2025");
    assert_eq!(Cell::Date("2025-03".into()).display(de), "März 2025");
    assert_eq!(Cell::Date("2025-03-15".into()).csv(), "2025-03-15");
    assert_eq!(Cell::Count(-1234).display(de), "\u{2212}1.234");
    assert_eq!(
        report_design::msat_of(&value("0.00000016473")),
        Cell::Msat(16473.into())
    );
    assert_eq!(
        report_design::msat_of(&value("0.000000000001")),
        Cell::Empty
    );
    assert_eq!(
        report_design::from_float(&Value::Float(1.6473e-07)),
        value("0.00000016473")
    );
}

#[test]
fn chart_axes_step_on_round_values() {
    use report_design::{Axis, Lang};
    let labels = |axis: &Axis| {
        axis.ticks
            .iter()
            .map(|(_, t)| t.clone())
            .collect::<Vec<_>>()
    };
    let axis = Axis::new(&[3_100.0, 51_234.0, 47_000.0], 4, Lang::En, true);
    assert_eq!((axis.low, axis.high), (0.0, 60_000.0));
    assert_eq!(labels(&axis), ["0", "20k", "40k", "60k"]);
    assert_eq!(axis.ticks.last().unwrap().0, 1.0);
    let axis = Axis::new(&[-12_000.0, 31_000.0], 4, Lang::De, true);
    assert_eq!(
        labels(&axis),
        ["\u{2212}20\u{a0}Tsd.", "0", "20\u{a0}Tsd.", "40\u{a0}Tsd."]
    );
    let axis = Axis::new(&[2_300_000.0], 4, Lang::De, true);
    assert_eq!(
        labels(&axis),
        ["0", "1\u{a0}Mio.", "2\u{a0}Mio.", "3\u{a0}Mio."]
    );
    let axis = Axis::new(&[0.4, 1.9], 4, Lang::De, false);
    assert_eq!(labels(&axis), ["0", "0,5", "1,0", "1,5", "2,0"]);
    let axis = Axis::new(&[0.4, 2.31], 4, Lang::En, false);
    assert_eq!(labels(&axis), ["0", "1", "2", "3"]);
    let axis = Axis::new(&[], 4, Lang::En, false);
    assert_eq!(labels(&axis), ["0", "0.25", "0.50", "0.75", "1.00"]);
}

#[test]
fn the_tax_country_picks_the_language_and_sparse_zeros_print_a_dash() {
    use report_design::{Block, Lang};
    let model = |country: &str| {
        kassiber_pytext::loads(&format!(
            r#"{{"summary": {{"fiat_currency": "EUR", "timeframe": {{"start": "2025-03-01T00:00:00Z", "end": "2025-03-31T00:00:00Z"}},
               "flow_periods": [{{"period": "2025-03", "inflow_volume": 1234.5, "outflow_volume": 0.0, "fees_fiat": 0.0, "fees_btc": 0.0}}],
               "realized_pnl_periods": [], "balance_history": []}},
               "profile": {{"tax_country": "{country}"}}, "kennzahlen": {{}}}}"#
        ))
        .unwrap()
    };
    let austrian = report_design::summary(&model("at"));
    let generic = report_design::summary(&model("generic"));
    assert_eq!((austrian.lang, generic.lang), (Lang::De, Lang::En));
    assert_eq!(austrian.title, "Übersichtsbericht");
    assert_eq!(austrian.period, "01.03.2025 bis 31.03.2025");
    assert_eq!(generic.title, "Summary report");
    assert_eq!(generic.period, "2025-03-01 to 2025-03-31");
    let months = |doc: &report_design::Document| {
        doc.sections
            .iter()
            .flat_map(|s| &s.blocks)
            .find_map(|b| match b {
                Block::Table(t) if t.id == "monthly_activity" => Some(t.clone()),
                _ => None,
            })
            .unwrap()
    };
    let table = months(&austrian);
    let row = table.display_row(&table.rows[0], Lang::De);
    assert_eq!(row[..4], ["März 2025", "1.234,50", "\u{2013}", "\u{2014}"]);
    assert_eq!(row[5..7], ["\u{2013}", "\u{2013}"]);
    let table = months(&generic);
    assert_eq!(table.columns[0].label, "Month");
    assert_eq!(
        table.display_row(&table.rows[0], Lang::En)[..3],
        ["2025-03", "1,234.50", "\u{2013}"]
    );
}

#[test]
fn formula_cells_name_their_own_row_and_cache_the_value() {
    use kassiber_xlsx::{Cell as XCell, Operation};
    use report_design::{Block, Cell, Column, Document, Kind, Lang, Section, Table};
    let value = |raw: &str| raw.parse::<kassiber_pydecimal::Decimal>().unwrap();
    let row = |a: &str, b: &str, check: &str| {
        vec![
            Cell::Btc(value(a)),
            Cell::Btc(value(b)),
            Cell::Formula {
                expression: "=ROUND({opening}-{closing},8)".into(),
                cached: Box::new(Cell::Btc(value(check))),
            },
        ]
    };
    let document = Document {
        kind: "test",
        title: "Test".into(),
        kicker: String::new(),
        book: String::new(),
        period: String::new(),
        period_start: String::new(),
        period_end: String::new(),
        generated_at: "2026-01-02T03:04:05Z".into(),
        input_version: None,
        fiat: "EUR".into(),
        country: String::new(),
        lang: Lang::En,
        sections: vec![Section {
            id: "checks",
            title: "Checks".into(),
            lead: String::new(),
            page_break: false,
            landscape: false,
            blocks: vec![Block::Table(Table {
                id: "checks",
                title: String::new(),
                columns: vec![
                    Column::new("opening", "Opening", Kind::Btc, 1.0),
                    Column::new("closing", "Closing", Kind::Btc, 1.0),
                    Column::new("check", "Check", Kind::Btc, 1.0),
                ],
                rows: vec![row("1", "0.5", "0.5"), row("2", "2", "0")],
                total: None,
                note: String::new(),
            })],
        }],
    };
    let workbook = report_design::xlsx_document(&document);
    let formulas: Vec<(u32, String, String)> = workbook.sheets[1]
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Cell {
                row,
                value: XCell::Formula { expression, result },
                ..
            } => Some((*row, expression.clone(), result.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        formulas,
        [
            (1, "=ROUND(A2-B2,8)".into(), "0.5".into()),
            (2, "=ROUND(A3-B3,8)".into(), "0.0".into())
        ]
    );
    // PDF and CSV show the cached value.
    assert_eq!(document_display(&document), "0.50000000");
}

fn document_display(document: &report_design::Document) -> String {
    match &document.sections[0].blocks[0] {
        report_design::Block::Table(t) => t.rows[0][2].display(report_design::Lang::En),
        _ => unreachable!(),
    }
}
