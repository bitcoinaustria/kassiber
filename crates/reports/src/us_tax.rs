//! The United States country layer's data: Form 8949 rows, the Schedule D
//! lines they feed, ordinary income by kind and network fees, per tax year
//! (`docs/reference/tax-country-us.md`). It reads the book's journal and
//! decides nothing the journal does not already say: terms, proceeds, basis
//! and gains are the tax engine's; this module groups and totals them and
//! places them on the forms.

use super::*;
use std::collections::BTreeMap;

/// The last tax year whose forms Kassiber knows; a later year is `unknown`.
pub const LAST_FORM_YEAR: i64 = 2026;
/// The first tax year with virtual currency guidance (Notice 2014-21).
pub const FIRST_FORM_YEAR: i64 = 2014;

/// Form 8949's boxes for transactions without a broker statement:
/// short-term and long-term. Digital assets use their own boxes from 2025.
pub fn boxes(year: i64) -> Option<(&'static str, &'static str)> {
    match year {
        FIRST_FORM_YEAR..=2024 => Some(("C", "F")),
        2025..=LAST_FORM_YEAR => Some(("I", "L")),
        _ => None,
    }
}

/// The holding period of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Term {
    Short,
    Long,
}

impl Term {
    pub fn code(self) -> &'static str {
        match self {
            Term::Short => "short",
            Term::Long => "long",
        }
    }
}

/// One Form 8949 row: one disposal's units of one holding period.
#[derive(Clone, Debug)]
pub struct Row {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    /// Units disposed of, in BTC.
    pub quantity: Decimal,
    /// Column (b): the acquisition date, or `VARIOUS` for several.
    pub acquired: String,
    /// Column (c), the UTC date.
    pub sold: String,
    pub proceeds: Decimal,
    pub cost: Decimal,
    pub gain: Decimal,
    pub term: Term,
}

/// One receipt of ordinary income.
#[derive(Clone, Debug)]
pub struct Income {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    pub date: String,
    pub kind: String,
    pub quantity: Decimal,
    pub value: Decimal,
}

/// Network fee units that left the book with their basis.
#[derive(Clone, Debug)]
pub struct Fee {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    pub date: String,
    pub quantity: Decimal,
    pub cost: Decimal,
}

/// One tax year's figures.
#[derive(Clone, Debug)]
pub struct Year {
    pub year: i64,
    pub rows: usize,
    pub short: [Decimal; 3],
    pub long: [Decimal; 3],
    pub income_by_kind: BTreeMap<String, Decimal>,
    pub income: Decimal,
    pub fee_cost: Decimal,
    pub fees: usize,
}

impl Year {
    fn new(year: i64) -> Self {
        Year {
            year,
            rows: 0,
            short: [Decimal::zero(), Decimal::zero(), Decimal::zero()],
            long: [Decimal::zero(), Decimal::zero(), Decimal::zero()],
            income_by_kind: BTreeMap::new(),
            income: Decimal::zero(),
            fee_cost: Decimal::zero(),
            fees: 0,
        }
    }
    pub fn boxes(&self) -> Option<(&'static str, &'static str)> {
        boxes(self.year)
    }
    /// Form 1040's digital asset question: any disposal, income or fee.
    pub fn digital_asset_question(&self) -> bool {
        self.rows > 0 || !self.income_by_kind.is_empty() || self.fees > 0
    }
}

fn year_of(date: &str) -> i64 {
    date.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0)
}

fn add(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.add(b, &DECIMAL).map_err(arithmetic)
}

/// The figures of every tax year the rows, income and fees fall in, in
/// year order.
pub fn years(rows: &[Row], income: &[Income], fees: &[Fee]) -> Result<Vec<Year>> {
    let mut years: BTreeMap<i64, Year> = BTreeMap::new();
    fn slot<'y>(years: &'y mut BTreeMap<i64, Year>, date: &str) -> &'y mut Year {
        let year = year_of(date);
        years.entry(year).or_insert_with(|| Year::new(year))
    }
    for row in rows {
        let y = slot(&mut years, &row.sold);
        y.rows += 1;
        let totals = match row.term {
            Term::Short => &mut y.short,
            Term::Long => &mut y.long,
        };
        totals[0] = add(&totals[0], &row.proceeds)?;
        totals[1] = add(&totals[1], &row.cost)?;
        totals[2] = add(&totals[2], &row.gain)?;
    }
    for item in income {
        let y = slot(&mut years, &item.date);
        let total = y
            .income_by_kind
            .entry(item.kind.clone())
            .or_insert_with(Decimal::zero);
        *total = add(total, &item.value)?;
        y.income = add(&y.income, &item.value)?;
    }
    for fee in fees {
        let y = slot(&mut years, &fee.date);
        y.fees += 1;
        y.fee_cost = add(&y.fee_cost, &fee.cost)?;
    }
    Ok(years.into_values().collect())
}

/// A fiat amount at cents, half-even, as the forms show it.
pub fn cents(value: &Decimal) -> Result<String> {
    value
        .to_fixed_places(2, Rounding::HalfEven)
        .map_err(arithmetic)
}

/// One year's figures in the shared vector schema's `years` form
/// (`docs/reference/tax-country-us-vectors.json`).
pub fn year_json(year: &Year) -> Result<serde_json::Value> {
    let boxes = year.boxes();
    let money = |d: &Decimal| cents(d).map(serde_json::Value::from);
    let mut income = serde_json::Map::new();
    for (kind, value) in &year.income_by_kind {
        income.insert(kind.clone(), money(value)?);
    }
    Ok(serde_json::json!({
        "status": if boxes.is_some() { "supported" } else { "unknown" },
        "form_8949_short_box": boxes.map(|b| b.0),
        "form_8949_long_box": boxes.map(|b| b.1),
        "short_proceeds": money(&year.short[0])?,
        "short_cost": money(&year.short[1])?,
        "short_gain": money(&year.short[2])?,
        "long_proceeds": money(&year.long[0])?,
        "long_cost": money(&year.long[1])?,
        "long_gain": money(&year.long[2])?,
        "schedule_d_line_3": if boxes.is_some() { money(&year.short[2])? } else { serde_json::Value::Null },
        "schedule_d_line_10": if boxes.is_some() { money(&year.long[2])? } else { serde_json::Value::Null },
        "ordinary_income": money(&year.income)?,
        "income_by_kind": income,
        "network_fee_cost": money(&year.fee_cost)?,
        "digital_asset_question": if year.digital_asset_question() { "yes" } else { "no" },
    }))
}

fn exact(row: &Value, key: &str) -> Result<Decimal> {
    let exact = value(row, &format!("{key}_exact"));
    if truthy(exact) {
        decimal(exact)
    } else {
        decimal(value(row, key))
    }
}

/// The UTC calendar date of a stored timestamp.
fn utc_date(raw: &str) -> String {
    match kassiber_pytext::iso_datetime(&raw.replace('Z', "+00:00")) {
        Some(dt) => {
            let days = dt.utc_us().div_euclid(86_400_000_000) - 719_162;
            civil(days)
        }
        None => raw.chars().take(10).collect(),
    }
}

/// `YYYY-MM-DD` of a day count from 1970-01-01.
fn civil(days: i64) -> String {
    // Howard Hinnant's days_from_civil inverse.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The book's rows, income and fees, from its journal, in journal order.
/// A `wallet` id limits them to that wallet.
pub fn load(
    conn: &Connection,
    profile_id: &str,
    wallet: Option<&str>,
) -> Result<(Vec<Row>, Vec<Income>, Vec<Fee>)> {
    let entries = rows(
        conn,
        "SELECT je.transaction_id,je.occurred_at,je.entry_type,je.asset,je.quantity,\
         je.proceeds,je.proceeds_exact,je.cost_basis,je.cost_basis_exact,je.gain_loss,je.gain_loss_exact,\
         je.capital_gains_type,je.calculation_json,je.calculation_compact,COALESCE(w.label,'') AS wallet,\
         COALESCE(NULLIF(t.kind_override,''),t.kind,'') AS kind \
         FROM journal_entries je LEFT JOIN wallets w ON w.id=je.wallet_id \
         LEFT JOIN transactions t ON t.id=je.transaction_id \
         WHERE je.profile_id=?1 AND (?2 IS NULL OR je.wallet_id=?2) \
         AND je.entry_type IN ('disposal','income','fee','transfer_fee') \
         ORDER BY je.occurred_at,je.created_at,je.id",
        kassiber_store::rusqlite::params![profile_id, wallet],
    )?;
    let (mut disposals, mut income, mut fees) = (Vec::new(), Vec::new(), Vec::new());
    for entry in &entries {
        let quantity = btc(int(entry, "quantity"))?.copy_abs();
        let date = utc_date(&text(entry, "occurred_at"));
        let transaction_id = text(entry, "transaction_id");
        let wallet = text(entry, "wallet");
        let asset = text(entry, "asset");
        match text(entry, "entry_type").as_str() {
            "income" => income.push(Income {
                transaction_id,
                wallet,
                asset,
                date,
                kind: kassiber_pytext::lower(kassiber_pytext::strip(&text(entry, "kind"))),
                quantity,
                value: exact(entry, "proceeds")?,
            }),
            "fee" | "transfer_fee" => fees.push(Fee {
                transaction_id,
                wallet,
                asset,
                date,
                quantity,
                cost: exact(entry, "cost_basis")?,
            }),
            _ => {
                // The lots' acquisition dates, from the fragments the engine
                // recorded; none readable reads as several (VARIOUS).
                let mut acquired: Vec<String> = Vec::new();
                if let Some(calculation) = crate::gain_explanation::calculation(entry) {
                    if let Value::List(fragments) = value(&calculation, "fragments") {
                        for fragment in fragments {
                            let at = value(value(fragment, "lot"), "occurred_at");
                            if truthy(at) {
                                let day = utc_date(&py_str(at));
                                if !acquired.contains(&day) {
                                    acquired.push(day);
                                }
                            }
                        }
                    }
                }
                disposals.push(Row {
                    transaction_id,
                    wallet,
                    asset,
                    quantity,
                    acquired: match acquired.as_slice() {
                        [single] => single.clone(),
                        _ => "VARIOUS".into(),
                    },
                    sold: date,
                    proceeds: exact(entry, "proceeds")?,
                    cost: exact(entry, "cost_basis")?,
                    gain: exact(entry, "gain_loss")?,
                    term: if text(entry, "capital_gains_type") == "long" {
                        Term::Long
                    } else {
                        Term::Short
                    },
                });
            }
        }
    }
    // Entries of one instant are stored in no particular order: rows read
    // by date, short-term first, then transaction, so a report repeats.
    disposals.sort_by(|a, b| {
        (&a.sold, a.term, &a.transaction_id, &a.acquired).cmp(&(
            &b.sold,
            b.term,
            &b.transaction_id,
            &b.acquired,
        ))
    });
    income.sort_by(|a, b| (&a.date, &a.transaction_id).cmp(&(&b.date, &b.transaction_id)));
    fees.sort_by(|a, b| (&a.date, &a.transaction_id).cmp(&(&b.date, &b.transaction_id)));
    Ok((disposals, income, fees))
}

fn dec(d: &Decimal) -> Value {
    s(d.to_string())
}

/// The layer's part of a report model: the rows, income, fees and years,
/// exact and unformatted.
pub fn model(conn: &Connection, profile: &Value, wallet: Option<&str>) -> Result<Value> {
    let (disposals, income, fees) = load(conn, &text(profile, "id"), wallet)?;
    let years = years(&disposals, &income, &fees)?;
    let row = |r: &Row| {
        dict([
            ("transaction_id", s(r.transaction_id.as_str())),
            ("wallet", s(r.wallet.as_str())),
            ("asset", s(r.asset.as_str())),
            ("quantity", dec(&r.quantity)),
            ("acquired", s(r.acquired.as_str())),
            ("sold", s(r.sold.as_str())),
            ("proceeds", dec(&r.proceeds)),
            ("cost", dec(&r.cost)),
            ("gain", dec(&r.gain)),
            ("term", s(r.term.code())),
        ])
    };
    Ok(dict([
        ("country", s("us")),
        (
            "pool_scope",
            s(kassiber_pytext::lower(kassiber_pytext::strip(&text(
                profile,
                "cost_basis_pool_scope",
            )))),
        ),
        ("wallet_scoped", Value::Bool(wallet.is_some())),
        ("rows", Value::List(disposals.iter().map(row).collect())),
        (
            "income",
            Value::List(
                income
                    .iter()
                    .map(|i| {
                        dict([
                            ("transaction_id", s(i.transaction_id.as_str())),
                            ("wallet", s(i.wallet.as_str())),
                            ("asset", s(i.asset.as_str())),
                            ("date", s(i.date.as_str())),
                            ("kind", s(i.kind.as_str())),
                            ("quantity", dec(&i.quantity)),
                            ("value", dec(&i.value)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "fees",
            Value::List(
                fees.iter()
                    .map(|f| {
                        dict([
                            ("transaction_id", s(f.transaction_id.as_str())),
                            ("wallet", s(f.wallet.as_str())),
                            ("asset", s(f.asset.as_str())),
                            ("date", s(f.date.as_str())),
                            ("quantity", dec(&f.quantity)),
                            ("cost", dec(&f.cost)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "years",
            Value::List(
                years
                    .iter()
                    .map(|y| {
                        let boxes = y.boxes();
                        dict([
                            ("year", Value::Int(y.year.into())),
                            (
                                "status",
                                s(if boxes.is_some() {
                                    "supported"
                                } else {
                                    "unknown"
                                }),
                            ),
                            ("short_box", boxes.map_or(Value::Null, |b| s(b.0))),
                            ("long_box", boxes.map_or(Value::Null, |b| s(b.1))),
                            ("rows", Value::Int(y.rows.into())),
                            ("short_proceeds", dec(&y.short[0])),
                            ("short_cost", dec(&y.short[1])),
                            ("short_gain", dec(&y.short[2])),
                            ("long_proceeds", dec(&y.long[0])),
                            ("long_cost", dec(&y.long[1])),
                            ("long_gain", dec(&y.long[2])),
                            ("income", dec(&y.income)),
                            ("fee_cost", dec(&y.fee_cost)),
                            (
                                "digital_asset_question",
                                Value::Bool(y.digital_asset_question()),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_follow_the_form_years() {
        assert_eq!(boxes(2013), None);
        assert_eq!(boxes(2014), Some(("C", "F")));
        assert_eq!(boxes(2024), Some(("C", "F")));
        assert_eq!(boxes(2025), Some(("I", "L")));
        assert_eq!(boxes(2026), Some(("I", "L")));
        assert_eq!(boxes(2027), None);
    }

    #[test]
    fn utc_dates_take_the_instant() {
        assert_eq!(utc_date("2024-06-17T22:00:00-05:00"), "2024-06-18");
        assert_eq!(utc_date("2024-02-29T23:59:59Z"), "2024-02-29");
        assert_eq!(utc_date("1969-12-31T23:00:00Z"), "1969-12-31");
        assert_eq!(civil(0), "1970-01-01");
        assert_eq!(civil(20_089), "2025-01-01");
    }

    #[test]
    fn years_total_by_term_and_kind() {
        let d = |v: &str| v.parse::<Decimal>().unwrap();
        let row = |sold: &str, term, p: &str, c: &str| Row {
            transaction_id: "t".into(),
            wallet: "A".into(),
            asset: "BTC".into(),
            quantity: d("1"),
            acquired: "2020-01-01".into(),
            sold: sold.into(),
            proceeds: d(p),
            cost: d(c),
            gain: d(p).sub(&d(c), &DECIMAL).unwrap(),
            term,
        };
        let rows = [
            row("2025-03-01", Term::Short, "100", "40"),
            row("2025-04-01", Term::Long, "50", "60"),
            row("2027-01-01", Term::Long, "5", "1"),
        ];
        let income = [Income {
            transaction_id: "i".into(),
            wallet: "A".into(),
            asset: "BTC".into(),
            date: "2025-05-01".into(),
            kind: "staking".into(),
            quantity: d("0.1"),
            value: d("12.345"),
        }];
        let years = years(&rows, &income, &[]).unwrap();
        assert_eq!(years.len(), 2);
        let json = year_json(&years[0]).unwrap();
        assert_eq!(json["short_gain"], "60.00");
        assert_eq!(json["long_gain"], "-10.00");
        assert_eq!(json["schedule_d_line_10"], "-10.00");
        assert_eq!(json["ordinary_income"], "12.34");
        assert_eq!(json["form_8949_short_box"], "I");
        let later = year_json(&years[1]).unwrap();
        assert_eq!(later["status"], "unknown");
        assert!(later["schedule_d_line_3"].is_null());
    }
}
