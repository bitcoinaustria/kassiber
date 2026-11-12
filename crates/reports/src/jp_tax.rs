//! Japan's country layer data: the NTA 暗号資産の計算書 per coin and Tokyo tax
//! year, the income receipts, the return figures (確定申告書 第一表 雑・その他)
//! and the review flags (`docs/reference/tax-country-jp.md`). It reads the
//! book's journal and decides nothing the journal does not already say:
//! each disposal's cost is the tax engine's average; this module adds the
//! year's movements up, rounds revenue down and expenses up to the yen as
//! the sheet does, and refuses the years whose regime is not settled.

use super::*;
use kassiber_tax_countries::calendar::{self, Date};
use kassiber_tax_countries::{tables, years::Lookup};
use std::collections::{BTreeMap, BTreeSet};

/// The cost method of the book (JP-R3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// 総平均法, the statutory default.
    TotalAverage,
    /// 移動平均法, by notification.
    MovingAverage,
}

impl Method {
    pub fn code(self) -> &'static str {
        match self {
            Method::TotalAverage => "total_average",
            Method::MovingAverage => "moving_average",
        }
    }
    /// The method of a stored `gains_algorithm`.
    pub fn of(stored: &str) -> Method {
        if stored.trim().eq_ignore_ascii_case("JP_MOVING_AVERAGE") {
            Method::MovingAverage
        } else {
            Method::TotalAverage
        }
    }
}

/// One acquisition: a purchase, a receipt, the coin received in an
/// exchange, or a fork's coins at zero.
#[derive(Clone, Debug)]
pub struct Acquisition {
    pub transaction_id: String,
    pub asset: String,
    pub us: i64,
    pub date: Date,
    pub quantity: Decimal,
    pub value: Decimal,
}

/// One disposal: a sale, a payment, an exchange, a gift, or fee units.
#[derive(Clone, Debug)]
pub struct Disposal {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    pub us: i64,
    pub date: Date,
    pub quantity: Decimal,
    pub revenue: Decimal,
    /// The engine's average cost of the units.
    pub cost: Decimal,
    /// Fees the disposal deducts (a sale's yen commission).
    pub expenses: Decimal,
    /// Network fee units of a move between own wallets: their value is
    /// revenue and an equal expense (JP-R8).
    pub network_fee: bool,
    /// Why the revenue is not the price (`seventy_percent`, JP-R13).
    pub valuation: Option<String>,
}

/// One income receipt (mining, staking, lending; JP-R9).
#[derive(Clone, Debug)]
pub struct Income {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    pub us: i64,
    pub date: Date,
    pub kind: String,
    pub quantity: Decimal,
    pub value: Decimal,
}

/// A row the journal could not book: its transaction, asset, date and
/// reason.
#[derive(Clone, Debug)]
pub struct Quarantine {
    pub transaction_id: String,
    pub asset: String,
    pub date: Date,
    pub reason: String,
}

fn add(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.add(b, &DECIMAL).map_err(arithmetic)
}
fn sub(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.sub(b, &DECIMAL).map_err(arithmetic)
}
fn div(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.div(b, &DECIMAL).map_err(arithmetic)
}
/// To the yen, rounded `rounding` (ROUNDDOWN or ROUNDUP of the sheet).
fn yen(value: &Decimal, rounding: Rounding) -> Result<Decimal> {
    value
        .quantize(0, &kassiber_pydecimal::Context::new(32, rounding))
        .map_err(arithmetic)
}

/// One coin's calculation sheet for one year (the NTA sheet's A–I and the
/// income it gives).
#[derive(Clone, Debug)]
pub struct Sheet {
    pub asset: String,
    pub opening_qty: Decimal,
    pub opening_value: Decimal,
    pub acquired_qty: Decimal,
    pub acquired_value: Decimal,
    pub unit_cost: Decimal,
    pub disposed_qty: Decimal,
    pub cost_of_disposals: Decimal,
    pub closing_qty: Decimal,
    pub closing_value: Decimal,
    pub revenue: Decimal,
    pub other_expenses: Decimal,
    pub revenue_rounded: Decimal,
    pub expenses_rounded: Decimal,
    pub income: Decimal,
}

/// One Tokyo tax year.
#[derive(Clone, Debug)]
pub struct Year {
    pub year: i32,
    pub sheets: Vec<Sheet>,
    /// Coins whose figures the year withholds (a disposal without known
    /// acquisitions, JP-R15).
    pub withheld: Vec<String>,
    pub income_receipts: BTreeMap<String, Decimal>,
    pub income_receipts_rounded: Decimal,
    /// その他雑所得 from crypto; `None` when a coin is withheld.
    pub total: Option<Decimal>,
}

impl Year {
    /// 第一表 収入金額等 雑・その他: revenue rounded plus receipts.
    pub fn return_revenue(&self) -> Result<Decimal> {
        let mut sum = self.income_receipts_rounded.clone();
        for sheet in &self.sheets {
            sum = add(&sum, &sheet.revenue_rounded)?;
        }
        Ok(sum)
    }
}

/// A year Kassiber refuses, with its reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub year: i32,
    pub reason: &'static str,
}

/// The reason a year is refused, if it is (JP-R16: from 2027 the regime is
/// not settled; a year before the table is unknown).
pub fn refused(year: i32) -> Option<&'static str> {
    match tables::JP_TAX_REGIME.at(Date::new(year, 1, 1).expect("1 January")) {
        Lookup::Value(_) => None,
        Lookup::Refused(code) => Some(code),
        Lookup::Unknown => Some("jp_tax_year_unknown"),
    }
}

/// A disposal's or receipt's reason that withholds its coin's year: the
/// units have no known acquisition (JP-R15, no automatic 5 % rule).
pub fn withholds(reason: &str) -> bool {
    matches!(reason, "insufficient_lots" | "missing_cost_basis")
}

/// Every Tokyo year with a movement or a quarantined row, the computed ones
/// in order and the refused ones apart.
pub fn years(
    method: Method,
    acquisitions: &[Acquisition],
    disposals: &[Disposal],
    income: &[Income],
    quarantines: &[Quarantine],
) -> Result<(Vec<Year>, Vec<Refusal>)> {
    let mut all: BTreeSet<i32> = BTreeSet::new();
    all.extend(acquisitions.iter().map(|a| a.date.year));
    all.extend(disposals.iter().map(|d| d.date.year));
    all.extend(income.iter().map(|i| i.date.year));
    all.extend(quarantines.iter().map(|q| q.date.year));
    let assets: BTreeSet<&str> = acquisitions
        .iter()
        .map(|a| a.asset.as_str())
        .chain(disposals.iter().map(|d| d.asset.as_str()))
        .collect();
    // Each coin's carried quantity, value and (moving average) unit cost.
    let mut carried: BTreeMap<&str, [Decimal; 3]> = BTreeMap::new();
    let (mut out, mut refusals) = (Vec::new(), Vec::new());
    let zero = Decimal::zero;
    for year in all {
        let withheld: Vec<String> = quarantines
            .iter()
            .filter(|q| q.date.year == year && withholds(&q.reason))
            .map(|q| q.asset.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut sheets = Vec::new();
        for &asset in &assets {
            let [opening_qty, opening_value, mut unit] = carried
                .get(asset)
                .cloned()
                .unwrap_or_else(|| [zero(), zero(), zero()]);
            // The year's movements in order, acquisitions first at one
            // instant.
            let mut moves: Vec<(i64, u8, usize)> = Vec::new();
            for (i, a) in acquisitions.iter().enumerate() {
                if a.asset == asset && a.date.year == year {
                    moves.push((a.us, 0, i));
                }
            }
            for (i, d) in disposals.iter().enumerate() {
                if d.asset == asset && d.date.year == year {
                    moves.push((d.us, 1, i));
                }
            }
            if moves.is_empty() && !opening_qty.is_positive() {
                continue;
            }
            moves.sort();
            let (mut acquired_qty, mut acquired_value) = (zero(), zero());
            let (mut disposed_qty, mut cost) = (zero(), zero());
            let (mut revenue, mut other) = (zero(), zero());
            let (mut quantity, mut book) = (opening_qty.clone(), opening_value.clone());
            for (_, kind, i) in moves {
                if kind == 0 {
                    let a = &acquisitions[i];
                    acquired_qty = add(&acquired_qty, &a.quantity)?;
                    acquired_value = add(&acquired_value, &a.value)?;
                    quantity = add(&quantity, &a.quantity)?;
                    book = add(&book, &a.value)?;
                    if quantity.is_positive() {
                        unit = div(&book, &quantity)?;
                    }
                } else {
                    let d = &disposals[i];
                    disposed_qty = add(&disposed_qty, &d.quantity)?;
                    cost = add(&cost, &d.cost)?;
                    quantity = sub(&quantity, &d.quantity)?;
                    book = sub(&book, &d.cost)?;
                    revenue = add(&revenue, &d.revenue)?;
                    other = add(&other, &d.expenses)?;
                    if d.network_fee {
                        other = add(&other, &d.revenue)?;
                    }
                }
            }
            let unit_cost = match method {
                Method::TotalAverage => {
                    let total = add(&opening_qty, &acquired_qty)?;
                    if total.is_positive() {
                        div(&add(&opening_value, &acquired_value)?, &total)?
                    } else {
                        zero()
                    }
                }
                Method::MovingAverage => unit.clone(),
            };
            let closing_qty = sub(&add(&opening_qty, &acquired_qty)?, &disposed_qty)?;
            let closing_value = sub(&add(&opening_value, &acquired_value)?, &cost)?;
            carried.insert(asset, [closing_qty.clone(), closing_value.clone(), unit]);
            let revenue_rounded = yen(&revenue, Rounding::Floor)?;
            let expenses_rounded = yen(&add(&cost, &other)?, Rounding::Ceiling)?;
            let income = sub(&revenue_rounded, &expenses_rounded)?;
            if withheld.iter().any(|w| w == asset) {
                continue;
            }
            sheets.push(Sheet {
                asset: asset.to_owned(),
                opening_qty,
                opening_value,
                acquired_qty,
                acquired_value,
                unit_cost,
                disposed_qty,
                cost_of_disposals: cost,
                closing_qty,
                closing_value,
                revenue,
                other_expenses: other,
                revenue_rounded,
                expenses_rounded,
                income,
            });
        }
        if let Some(reason) = refused(year) {
            refusals.push(Refusal { year, reason });
            continue;
        }
        let mut receipts: BTreeMap<String, Decimal> = BTreeMap::new();
        let mut receipts_total = zero();
        for item in income.iter().filter(|i| i.date.year == year) {
            let slot = receipts.entry(item.kind.clone()).or_insert_with(zero);
            *slot = add(slot, &item.value)?;
            receipts_total = add(&receipts_total, &item.value)?;
        }
        let income_receipts_rounded = yen(&receipts_total, Rounding::Floor)?;
        let total = if withheld.is_empty() {
            let mut total = income_receipts_rounded.clone();
            for sheet in &sheets {
                total = add(&total, &sheet.income)?;
            }
            Some(total)
        } else {
            None
        };
        out.push(Year {
            year,
            sheets,
            withheld,
            income_receipts: receipts,
            income_receipts_rounded,
            total,
        });
    }
    Ok((out, refusals))
}

/// The revenue above which crypto gains may be business or business-like
/// miscellaneous income (JP-R2, FAQ 2-2).
pub const CLASSIFICATION_REVENUE: i64 = 3_000_000;

/// A review flag on a disposal (`event` its transaction), a coin's year
/// (`2025:BTC`), or a year (`2025`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    pub event: String,
    pub code: &'static str,
}

/// The year's flags: disposals below 70 % of market value, coins whose
/// revenue passes ¥3,000,000, and a net loss that offsets nothing.
pub fn flags(years: &[Year], disposals: &[Disposal]) -> Vec<Flag> {
    let threshold = Decimal::from(CLASSIFICATION_REVENUE);
    let mut flags = Vec::new();
    for year in years {
        for d in disposals.iter().filter(|d| d.date.year == year.year) {
            if d.valuation.as_deref() == Some("seventy_percent") {
                flags.push(Flag {
                    event: d.transaction_id.clone(),
                    code: "jp_low_value_transfer_70pct",
                });
            }
        }
        for sheet in &year.sheets {
            if sheet.revenue > threshold {
                flags.push(Flag {
                    event: format!("{}:{}", year.year, sheet.asset),
                    code: "jp_revenue_over_3m_classification",
                });
            }
        }
        if year.total.as_ref().is_some_and(Decimal::is_negative) {
            flags.push(Flag {
                event: year.year.to_string(),
                code: "jp_zatsu_loss_no_offset",
            });
        }
    }
    flags
}

/// A decimal as the vectors write it: plain, without trailing zeros.
pub fn number(value: &Decimal) -> String {
    let text = value
        .to_fixed_string()
        .unwrap_or_else(|_| value.to_canonical());
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    if text == "-0" {
        "0".into()
    } else {
        text
    }
}

/// One year's figures in the shared vector schema's `years` form
/// (`docs/reference/tax-country-jp-vectors.json`).
pub fn year_json(year: &Year, method: Method) -> serde_json::Value {
    let mut assets = serde_json::Map::new();
    for sheet in &year.sheets {
        assets.insert(
            sheet.asset.clone(),
            serde_json::json!({
                "opening_qty": number(&sheet.opening_qty),
                "opening_value": number(&sheet.opening_value),
                "acquired_qty": number(&sheet.acquired_qty),
                "acquired_value": number(&sheet.acquired_value),
                "unit_cost": number(&sheet.unit_cost),
                "disposed_qty": number(&sheet.disposed_qty),
                "cost_of_disposals": number(&sheet.cost_of_disposals),
                "closing_qty": number(&sheet.closing_qty),
                "closing_value": number(&sheet.closing_value),
                "revenue": number(&sheet.revenue),
                "other_expenses": number(&sheet.other_expenses),
                "revenue_rounded": number(&sheet.revenue_rounded),
                "expenses_rounded": number(&sheet.expenses_rounded),
                "income": number(&sheet.income),
            }),
        );
    }
    let receipts: serde_json::Map<String, serde_json::Value> = year
        .income_receipts
        .iter()
        .map(|(kind, value)| (kind.clone(), number(value).into()))
        .collect();
    serde_json::json!({
        "method": method.code(),
        "assets": assets,
        "income_receipts": receipts,
        "income_receipts_rounded": number(&year.income_receipts_rounded),
        "zatsu_crypto_total": year.total.as_ref().map(number),
    })
}

/// The instant (µs) of a stored timestamp, and its Tokyo date.
fn instant(raw: &str, zone: &calendar::Zone) -> Result<(i64, Date)> {
    let dt = kassiber_pytext::iso_datetime(&raw.replace('Z', "+00:00")).ok_or_else(|| {
        AppError::new(
            "internal_error",
            format!(
                "Invalid isoformat string: {}",
                kassiber_pytext::repr_str(raw)
            ),
        )
    })?;
    // Microseconds since 1970-01-01, as the registry's calendar counts.
    let us = dt.utc_us() - kassiber_rates::time::UNIX;
    let date = zone
        .local_date(us)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    Ok((us, date))
}

fn exact(row: &Value, key: &str) -> Result<Decimal> {
    let exact = value(row, &format!("{key}_exact"));
    if truthy(exact) {
        decimal(exact)
    } else {
        decimal(value(row, key))
    }
}

/// The book's journal: acquisitions, disposals, receipts, and the rows it
/// quarantined, in date order. A `wallet` id limits them to that wallet.
#[allow(clippy::type_complexity)]
pub fn load(
    conn: &Connection,
    profile_id: &str,
    wallet: Option<&str>,
) -> Result<(
    Vec<Acquisition>,
    Vec<Disposal>,
    Vec<Income>,
    Vec<Quarantine>,
)> {
    let zone = calendar::zone(kassiber_tax_countries::TOKYO).expect("Tokyo has a calendar");
    let entries = rows(
        conn,
        "SELECT je.transaction_id,je.occurred_at,je.entry_type,je.asset,je.quantity,\
         je.proceeds,je.proceeds_exact,je.cost_basis,je.cost_basis_exact,\
         je.fiat_value,je.calculation_json,je.calculation_compact,\
         COALESCE(w.label,'') AS wallet,\
         COALESCE(NULLIF(t.kind_override,''),t.kind,'') AS kind \
         FROM journal_entries je LEFT JOIN wallets w ON w.id=je.wallet_id \
         LEFT JOIN transactions t ON t.id=je.transaction_id \
         WHERE je.profile_id=?1 AND (?2 IS NULL OR je.wallet_id=?2) \
         AND je.entry_type IN ('acquisition','disposal','income','fee','transfer_fee') \
         ORDER BY je.occurred_at,je.created_at,je.id",
        kassiber_store::rusqlite::params![profile_id, wallet],
    )?;
    let (mut acquisitions, mut disposals, mut income) = (Vec::new(), Vec::new(), Vec::new());
    for entry in &entries {
        let (us, date) = instant(&text(entry, "occurred_at"), &zone)?;
        let transaction_id = text(entry, "transaction_id");
        let asset = text(entry, "asset");
        let quantity = btc(int(entry, "quantity"))?.copy_abs();
        match text(entry, "entry_type").as_str() {
            "acquisition" => acquisitions.push(Acquisition {
                transaction_id,
                asset,
                us,
                date,
                quantity,
                value: decimal(value(entry, "fiat_value"))?,
            }),
            "income" => income.push(Income {
                transaction_id,
                wallet: text(entry, "wallet"),
                asset,
                us,
                date,
                kind: kassiber_pytext::lower(kassiber_pytext::strip(&text(entry, "kind"))),
                quantity,
                value: exact(entry, "proceeds")?,
            }),
            entry_type => {
                let calculation = crate::gain_explanation::calculation(entry);
                let fact = |key: &str| {
                    calculation
                        .as_ref()
                        .map_or(&Value::Null, |calculation| value(calculation, key))
                };
                disposals.push(Disposal {
                    transaction_id,
                    wallet: text(entry, "wallet"),
                    asset,
                    us,
                    date,
                    quantity,
                    revenue: exact(entry, "proceeds")?,
                    cost: exact(entry, "cost_basis")?,
                    expenses: if truthy(fact("expenses_exact")) {
                        decimal(fact("expenses_exact"))?
                    } else {
                        Decimal::zero()
                    },
                    network_fee: entry_type == "transfer_fee",
                    valuation: truthy(fact("valuation")).then(|| py_str(fact("valuation"))),
                });
            }
        }
    }
    let mut quarantines = Vec::new();
    for row in rows(
        conn,
        "SELECT q.transaction_id,q.reason,t.asset,t.occurred_at FROM journal_quarantines q \
         JOIN transactions t ON t.id=q.transaction_id \
         WHERE q.profile_id=?1 AND (?2 IS NULL OR t.wallet_id=?2) ORDER BY t.occurred_at,q.transaction_id",
        kassiber_store::rusqlite::params![profile_id, wallet],
    )? {
        let (_, date) = instant(&text(&row, "occurred_at"), &zone)?;
        quarantines.push(Quarantine {
            transaction_id: text(&row, "transaction_id"),
            asset: text(&row, "asset"),
            date,
            reason: text(&row, "reason"),
        });
    }
    let order = |us: i64, id: &str| (us, id.to_owned());
    acquisitions.sort_by_key(|a| order(a.us, &a.transaction_id));
    disposals.sort_by_key(|d| order(d.us, &d.transaction_id));
    income.sort_by_key(|i| order(i.us, &i.transaction_id));
    Ok((acquisitions, disposals, income, quarantines))
}

fn dec(d: &Decimal) -> Value {
    s(d.to_string())
}

/// The layer's part of a report model.
pub fn model(conn: &Connection, profile: &Value, wallet: Option<&str>) -> Result<Value> {
    let method = Method::of(&text(profile, "gains_algorithm"));
    let (acquisitions, disposals, income, quarantines) = load(conn, &text(profile, "id"), wallet)?;
    let (years, refusals) = years(method, &acquisitions, &disposals, &income, &quarantines)?;
    let flags = flags(&years, &disposals);
    // The venues of each year: the wallets its disposals and receipts name.
    let venues = |year: i32| -> String {
        let names: BTreeSet<&str> = disposals
            .iter()
            .filter(|d| d.date.year == year)
            .map(|d| d.wallet.as_str())
            .chain(
                income
                    .iter()
                    .filter(|i| i.date.year == year)
                    .map(|i| i.wallet.as_str()),
            )
            .collect();
        names.into_iter().collect::<Vec<_>>().join(", ")
    };
    let mut year_values = Vec::new();
    for y in &years {
        let sheets = y
            .sheets
            .iter()
            .map(|sheet| {
                dict([
                    ("asset", s(sheet.asset.as_str())),
                    ("opening_qty", dec(&sheet.opening_qty)),
                    ("opening_value", dec(&sheet.opening_value)),
                    ("acquired_qty", dec(&sheet.acquired_qty)),
                    ("acquired_value", dec(&sheet.acquired_value)),
                    ("unit_cost", dec(&sheet.unit_cost)),
                    ("disposed_qty", dec(&sheet.disposed_qty)),
                    ("cost_of_disposals", dec(&sheet.cost_of_disposals)),
                    ("closing_qty", dec(&sheet.closing_qty)),
                    ("closing_value", dec(&sheet.closing_value)),
                    ("revenue", dec(&sheet.revenue)),
                    ("other_expenses", dec(&sheet.other_expenses)),
                    ("revenue_rounded", dec(&sheet.revenue_rounded)),
                    ("expenses_rounded", dec(&sheet.expenses_rounded)),
                    ("income", dec(&sheet.income)),
                ])
            })
            .collect();
        let return_income = match &y.total {
            Some(total) if total.is_negative() => Value::Str(Decimal::zero().to_string().into()),
            Some(total) => dec(total),
            None => Value::Null,
        };
        year_values.push(dict([
            ("year", Value::Int(y.year.into())),
            ("sheets", Value::List(sheets)),
            ("return_revenue", dec(&y.return_revenue()?)),
            ("income_receipts_rounded", dec(&y.income_receipts_rounded)),
            ("total", y.total.as_ref().map_or(Value::Null, dec)),
            ("return_income", return_income),
            ("venues", s(venues(y.year))),
        ]));
    }
    Ok(dict([
        ("country", s("jp")),
        ("method", s(method.code())),
        ("wallet_scoped", Value::Bool(wallet.is_some())),
        ("years", Value::List(year_values)),
        (
            "disposals",
            Value::List(
                disposals
                    .iter()
                    .map(|d| {
                        dict([
                            ("transaction_id", s(d.transaction_id.as_str())),
                            ("wallet", s(d.wallet.as_str())),
                            ("asset", s(d.asset.as_str())),
                            ("year", Value::Int(d.date.year.into())),
                            ("date", s(d.date.iso())),
                            ("quantity", dec(&d.quantity)),
                            ("revenue", dec(&d.revenue)),
                            ("cost", dec(&d.cost)),
                            ("expenses", dec(&d.expenses)),
                            ("valuation", s(d.valuation.as_deref().unwrap_or(""))),
                        ])
                    })
                    .collect(),
            ),
        ),
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
                            ("date", s(i.date.iso())),
                            ("kind", s(i.kind.as_str())),
                            ("quantity", dec(&i.quantity)),
                            ("value", dec(&i.value)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "flags",
            Value::List(
                flags
                    .iter()
                    .map(|flag| dict([("event", s(flag.event.as_str())), ("code", s(flag.code))]))
                    .collect(),
            ),
        ),
        (
            "refusals",
            Value::List(
                refusals
                    .iter()
                    .map(|r| dict([("year", Value::Int(r.year.into())), ("reason", s(r.reason))]))
                    .collect(),
            ),
        ),
    ]))
}
