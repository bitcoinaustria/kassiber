//! The German country layer's data: § 23 EStG private disposals per
//! consumed lot, § 22 Nr. 3 income and block rewards, each year's Freigrenze
//! verdicts and the Anlage SO lines they go on
//! (`docs/reference/tax-country-de.md`). Holding periods, proceeds, costs,
//! Werbungskosten and gains are the tax engine's (`fee_units =
//! disposal_before_sale`, wallet pools, Berlin dates); this module groups
//! and totals them, applies the dated Freigrenzen (`tables.rs`) and places
//! them on the form.

use super::*;
use crate::tax_years::{exemption_limit, table_value, Limit};
use kassiber_tax_countries::calendar::Date;
use kassiber_tax_countries::tables;
use kassiber_tax_countries::years::Schedule;
use std::collections::BTreeMap;

/// § 22 Nr. 3 EStG income.
pub const P22_3: &str = "p22_3";
/// Block rewards awaiting classification (§ 15 or § 22 Nr. 3).
pub const MINING: &str = "mining_classification_required";

/// The section a receipt of `kind` belongs to: block rewards are reported
/// apart until classified (DE-INC-MINING); everything else the journal
/// books as income is § 22 Nr. 3 (staking, lending, routing; DE-INC-LENDING,
/// DE-INC-ROUTING).
pub fn section(kind: &str) -> &'static str {
    match kind {
        "mining" | "mining_reward" => MINING,
        _ => P22_3,
    }
}

/// One consumed lot of a disposal.
#[derive(Clone, Debug)]
pub struct Fragment {
    /// The acquiring transaction.
    pub lot: String,
    /// The acquisition's Berlin date, or empty when the journal has none.
    pub acquired: String,
    pub quantity: Decimal,
    pub cost: Decimal,
    pub proceeds: Decimal,
    /// Werbungskosten (DE-FEE-DISP, DE-FEE-SPLIT).
    pub expenses: Decimal,
    /// Net of the Werbungskosten.
    pub gain: Decimal,
    /// Inside the one-year period (§ 23 Abs. 1 Satz 1 Nr. 2).
    pub taxable: bool,
}

/// One disposal: a sale, a spend, a swap's given leg, or fee units.
#[derive(Clone, Debug)]
pub struct Disposal {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    /// `sell`, `spend`, `swap`, ... or `fee`.
    pub kind: String,
    /// The Berlin date.
    pub date: String,
    pub fragments: Vec<Fragment>,
}

/// One receipt the journal books as income.
#[derive(Clone, Debug)]
pub struct Income {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    /// The Berlin date.
    pub date: String,
    pub kind: String,
    pub quantity: Decimal,
    pub value: Decimal,
    pub section: &'static str,
}

/// A year's figure against its Freigrenze.
#[derive(Clone, Debug)]
pub struct Verdict {
    pub limit: Decimal,
    pub result: Limit,
    pub amount: Decimal,
}

/// One tax year's figures.
#[derive(Clone, Debug)]
pub struct Year {
    pub year: i32,
    /// Taxable fragments' proceeds, costs, Werbungskosten and net gain.
    pub taxable: [Decimal; 4],
    /// Gains and losses outside the one-year period.
    pub outside: Decimal,
    /// The § 23 verdict; `None` for a year without a Freigrenze on file
    /// (never the neighbouring year's).
    pub p23: Option<Verdict>,
    pub p22_3: Decimal,
    pub p22_3_verdict: Option<Verdict>,
    pub mining: Decimal,
    pub disposals: usize,
}

impl Year {
    fn new(year: i32) -> Self {
        Year {
            year,
            taxable: [
                Decimal::zero(),
                Decimal::zero(),
                Decimal::zero(),
                Decimal::zero(),
            ],
            outside: Decimal::zero(),
            p23: None,
            p22_3: Decimal::zero(),
            p22_3_verdict: None,
            mining: Decimal::zero(),
            disposals: 0,
        }
    }
}

/// The Anlage SO line numbers of a year on file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lines {
    /// Leistungen (§ 22 Nr. 3).
    pub leistungen: &'static str,
    /// Kryptowerte (§ 23).
    pub kryptowerte: &'static str,
    /// The loss deduction (Verlustabzug).
    pub loss: &'static str,
}

/// The Anlage SO line numbers of `year`; other years print the field names
/// only.
pub fn lines(year: i32) -> Option<Lines> {
    match year {
        2024 => Some(Lines {
            leistungen: "10–16",
            kryptowerte: "41–47, 54–55",
            loss: "62",
        }),
        // The 2025 ELSTER help names both line 43 and line 45 as the start
        // of the crypto block (open question Q1).
        2025 => Some(Lines {
            leistungen: "14–21",
            kryptowerte: "43/45–49, 56–57",
            loss: "66",
        }),
        _ => None,
    }
}

fn add(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.add(b, &DECIMAL).map_err(arithmetic)
}

fn verdict(table: &Schedule, year: i32, total: &Decimal) -> Result<Option<Verdict>> {
    let first = Date::new(year, 1, 1)
        .ok_or_else(|| AppError::new("internal_error", "date value out of range"))?;
    Ok(table_value(table, first)?.ok().map(|limit| {
        let (result, amount) = exemption_limit(total, &limit);
        Verdict {
            limit,
            result,
            amount,
        }
    }))
}

fn year_of(date: &str) -> i32 {
    date.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0)
}

/// The figures of every tax year the disposals and income fall in, in
/// year order. The § 23 Freigrenze is tested on the year's net figure
/// before any loss carry (BFH IX R 27/04).
pub fn years(disposals: &[Disposal], income: &[Income]) -> Result<Vec<Year>> {
    let mut years: BTreeMap<i32, Year> = BTreeMap::new();
    for disposal in disposals {
        let year = year_of(&disposal.date);
        let y = years.entry(year).or_insert_with(|| Year::new(year));
        y.disposals += 1;
        for fragment in &disposal.fragments {
            if fragment.taxable {
                y.taxable[0] = add(&y.taxable[0], &fragment.proceeds)?;
                y.taxable[1] = add(&y.taxable[1], &fragment.cost)?;
                y.taxable[2] = add(&y.taxable[2], &fragment.expenses)?;
                y.taxable[3] = add(&y.taxable[3], &fragment.gain)?;
            } else {
                y.outside = add(&y.outside, &fragment.gain)?;
            }
        }
    }
    for item in income {
        let year = year_of(&item.date);
        let y = years.entry(year).or_insert_with(|| Year::new(year));
        if item.section == MINING {
            y.mining = add(&y.mining, &item.value)?;
        } else {
            y.p22_3 = add(&y.p22_3, &item.value)?;
        }
    }
    for y in years.values_mut() {
        y.p23 = verdict(&tables::DE_P23_FREIGRENZE, y.year, &y.taxable[3])?;
        y.p22_3_verdict = verdict(&tables::DE_P22_3_FREIGRENZE, y.year, &y.p22_3)?;
    }
    Ok(years.into_values().collect())
}

/// A verdict's name: `none`, `loss`, `exempt_freigrenze`, `taxable`, or
/// `unknown` for a year without a table.
pub fn result_name(verdict: Option<&Verdict>) -> &'static str {
    match verdict.map(|v| v.result) {
        None => "unknown",
        Some(Limit::None) => "none",
        Some(Limit::Loss) => "loss",
        Some(Limit::Exempt) => "exempt_freigrenze",
        Some(Limit::Taxable) => "taxable",
    }
}

/// A fiat amount at cents, half-even.
pub fn cents(value: &Decimal) -> Result<String> {
    value
        .to_fixed_places(2, Rounding::HalfEven)
        .map_err(arithmetic)
}

/// One year's figures in the shared vector schema's `years` form
/// (`docs/reference/tax-country-de-vectors.json`); an unknown year's limit
/// and amount are `null`.
pub fn year_json(year: &Year) -> Result<serde_json::Value> {
    let money = |d: &Decimal| cents(d).map(serde_json::Value::from);
    let limit = |v: Option<&Verdict>| v.map_or(Ok(serde_json::Value::Null), |v| money(&v.limit));
    let amount = |v: Option<&Verdict>| v.map_or(Ok(serde_json::Value::Null), |v| money(&v.amount));
    Ok(serde_json::json!({
        "p23_taxable_proceeds": money(&year.taxable[0])?,
        "p23_taxable_cost": money(&year.taxable[1])?,
        "p23_taxable_expenses": money(&year.taxable[2])?,
        "p23_net_gain": money(&year.taxable[3])?,
        "p23_freigrenze": limit(year.p23.as_ref())?,
        "p23_result": result_name(year.p23.as_ref()),
        "p23_amount": amount(year.p23.as_ref())?,
        "exempt_gain_outside_period": money(&year.outside)?,
        "p22_3_total": money(&year.p22_3)?,
        "p22_3_result": result_name(year.p22_3_verdict.as_ref()),
        "p22_3_amount": amount(year.p22_3_verdict.as_ref())?,
        "mining_classification_required_total": money(&year.mining)?,
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

/// The book's Berlin date of a stored timestamp.
fn local_date(raw: &str, zone: &str) -> Result<String> {
    Ok(crate::calendar::local(raw, zone)?.1)
}

fn kind_of(raw: &str) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(raw)).replace(['-', ' '], "_")
}

/// The book's disposals and income from its journal, in journal order;
/// `wallet` limits them to one wallet. Dates are local dates in the
/// country's zone. A fragment's Werbungskosten are the stored
/// `expenses_exact`, or what its proceeds exceed its cost and gain by when
/// an older journal has none.
pub fn load(
    conn: &Connection,
    profile_id: &str,
    zone: &str,
    wallet: Option<&str>,
) -> Result<(Vec<Disposal>, Vec<Income>)> {
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
    let (mut disposals, mut income) = (Vec::<Disposal>::new(), Vec::new());
    for entry in &entries {
        let quantity = btc(int(entry, "quantity"))?.copy_abs();
        let date = local_date(&text(entry, "occurred_at"), zone)?;
        let transaction_id = text(entry, "transaction_id");
        let wallet = text(entry, "wallet");
        let asset = text(entry, "asset");
        let entry_type = text(entry, "entry_type");
        if entry_type == "income" {
            let kind = kind_of(&text(entry, "kind"));
            income.push(Income {
                transaction_id,
                wallet,
                asset,
                date,
                section: section(&kind),
                kind,
                quantity,
                value: exact(entry, "proceeds")?,
            });
            continue;
        }
        let taxable = text(entry, "capital_gains_type") != "long";
        let mut fragments = Vec::new();
        let calculation = crate::gain_explanation::calculation(entry);
        if let Some(Value::List(stored)) = calculation.as_ref().map(|c| value(c, "fragments")) {
            for fragment in stored {
                let lot = value(fragment, "lot");
                let at = value(lot, "occurred_at");
                let figure = |key: &str| decimal(value(fragment, &format!("{key}_exact")));
                let (cost, proceeds, gain) = (
                    figure("cost_basis")?,
                    figure("proceeds")?,
                    figure("gain_loss")?,
                );
                let expenses = match value(fragment, "expenses_exact") {
                    Value::Null => proceeds
                        .sub(&cost, &DECIMAL)
                        .and_then(|d| d.sub(&gain, &DECIMAL))
                        .map_err(arithmetic)?,
                    stored => decimal(stored)?,
                };
                let msat = match value(fragment, "quantity_msat") {
                    Value::Int(n) => i64::try_from(n.clone()).unwrap_or(0),
                    _ => 0,
                };
                fragments.push(Fragment {
                    lot: text(lot, "transaction_id"),
                    acquired: if truthy(at) {
                        local_date(&py_str(at), zone)?
                    } else {
                        String::new()
                    },
                    quantity: btc(msat)?.copy_abs(),
                    cost,
                    proceeds,
                    expenses,
                    gain,
                    taxable,
                });
            }
        }
        if fragments.is_empty() {
            // No readable fragments: the entry as one, its lot unknown.
            let (cost, proceeds, gain) = (
                exact(entry, "cost_basis")?,
                exact(entry, "proceeds")?,
                exact(entry, "gain_loss")?,
            );
            fragments.push(Fragment {
                lot: String::new(),
                acquired: String::new(),
                quantity: quantity.clone(),
                expenses: proceeds
                    .sub(&cost, &DECIMAL)
                    .and_then(|d| d.sub(&gain, &DECIMAL))
                    .map_err(arithmetic)?,
                cost,
                proceeds,
                gain,
                taxable,
            });
        }
        let kind = if entry_type == "disposal" {
            kind_of(&text(entry, "kind"))
        } else {
            "fee".to_owned()
        };
        // A disposal spanning both sides of the period is two entries.
        match disposals.iter_mut().find(|d| {
            d.transaction_id == transaction_id
                && d.kind == kind
                && d.wallet == wallet
                && d.asset == asset
        }) {
            Some(disposal) => disposal.fragments.extend(fragments),
            None => disposals.push(Disposal {
                transaction_id,
                wallet,
                asset,
                kind,
                date,
                fragments,
            }),
        }
    }
    // A sale's fee units are disposed of before it (DE-FEE-NETWORK).
    disposals.sort_by(|a, b| (&a.date, a.kind != "fee").cmp(&(&b.date, b.kind != "fee")));
    // Entries of one instant are stored in no particular order: fragments
    // read by acquisition date, outside the period first, so a report repeats.
    for disposal in &mut disposals {
        disposal.fragments.sort_by(|a, b| {
            (&a.acquired, a.taxable, &a.lot).cmp(&(&b.acquired, b.taxable, &b.lot))
        });
    }
    Ok((disposals, income))
}

/// Each wallet's holding of each asset at the end of every year, by the
/// journal's quantities in the country's zone: `(year, wallet, asset,
/// quantity)` for the positive ones.
pub fn year_end_holdings(
    conn: &Connection,
    profile_id: &str,
    zone: &str,
    years: &[i32],
    wallet: Option<&str>,
) -> Result<Vec<(i32, String, String, Decimal)>> {
    let entries = rows(
        conn,
        "SELECT je.occurred_at,je.entry_type,je.asset,je.quantity,COALESCE(w.label,'') AS wallet \
         FROM journal_entries je LEFT JOIN wallets w ON w.id=je.wallet_id \
         WHERE je.profile_id=?1 AND (?2 IS NULL OR je.wallet_id=?2) \
         ORDER BY je.occurred_at,je.created_at,je.id",
        kassiber_store::rusqlite::params![profile_id, wallet],
    )?;
    let mut dated = Vec::new();
    for entry in &entries {
        // An income entry's units are its acquisition's; a move's fee is in
        // its outbound leg (as the year handoff counts them).
        if ["income", "transfer_fee"].contains(&text(entry, "entry_type").as_str()) {
            continue;
        }
        let year = crate::calendar::local(&text(entry, "occurred_at"), zone)?.0 as i32;
        dated.push((
            year,
            text(entry, "wallet"),
            text(entry, "asset"),
            btc(int(entry, "quantity"))?,
        ));
    }
    let mut out = Vec::new();
    for &year in years {
        let mut totals: BTreeMap<(String, String), Decimal> = BTreeMap::new();
        for (at, wallet, asset, quantity) in &dated {
            if *at <= year {
                let slot = totals
                    .entry((wallet.clone(), asset.clone()))
                    .or_insert_with(Decimal::zero);
                *slot = add(slot, quantity)?;
            }
        }
        for ((wallet, asset), quantity) in totals {
            if quantity.is_positive() {
                out.push((year, wallet, asset, quantity));
            }
        }
    }
    Ok(out)
}

/// How many disposal and income entries the journal priced at each
/// granularity (Rn. 90: the report states its price settings), in name
/// order; rows without one count as `unknown`.
pub fn price_granularity(conn: &Connection, profile_id: &str) -> Result<Vec<(String, i64)>> {
    let counted = rows(
        conn,
        "SELECT COALESCE(NULLIF(t.pricing_granularity,''),'unknown') AS granularity,COUNT(DISTINCT t.id) AS n \
         FROM journal_entries je JOIN transactions t ON t.id=je.transaction_id \
         WHERE je.profile_id=? AND je.entry_type IN ('disposal','income','fee','transfer_fee') \
         GROUP BY 1 ORDER BY 1",
        [profile_id],
    )?;
    Ok(counted
        .iter()
        .map(|row| (text(row, "granularity"), int(row, "n")))
        .collect())
}

fn dec(d: &Decimal) -> Value {
    s(d.to_string())
}

/// The layer's part of a report model: the disposals with their
/// fragments, the income, the years and the year-end holdings, exact and
/// unformatted.
pub fn model(conn: &Connection, profile: &Value, wallet: Option<&str>) -> Result<Value> {
    let profile_id = text(profile, "id");
    let country = crate::country::entry(&kassiber_pytext::lower(kassiber_pytext::strip(&text(
        profile,
        "tax_country",
    ))))
    .ok_or_else(|| AppError::new("internal_error", "Unsupported tax country"))?;
    let zone = country.timezone;
    let (disposals, income) = load(conn, &profile_id, zone, wallet)?;
    let years = years(&disposals, &income)?;
    let mut holding_years: Vec<i32> = Vec::new();
    for year in &years {
        for y in [year.year - 1, year.year] {
            if !holding_years.contains(&y) {
                holding_years.push(y);
            }
        }
    }
    holding_years.sort_unstable();
    let holdings = year_end_holdings(conn, &profile_id, zone, &holding_years, wallet)?;
    let verdict = |v: Option<&Verdict>| -> Value {
        match v {
            Some(v) => dict([
                ("limit", dec(&v.limit)),
                ("result", s(result_name(Some(v)))),
                ("amount", dec(&v.amount)),
            ]),
            None => dict([("result", s("unknown"))]),
        }
    };
    Ok(dict([
        ("country", s(country.code)),
        ("wallet_scoped", Value::Bool(wallet.is_some())),
        (
            "method",
            s(kassiber_pytext::upper(kassiber_pytext::strip(&text(
                profile,
                "gains_algorithm",
            )))),
        ),
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
                            ("kind", s(d.kind.as_str())),
                            ("date", s(d.date.as_str())),
                            (
                                "fragments",
                                Value::List(
                                    d.fragments
                                        .iter()
                                        .map(|f| {
                                            dict([
                                                ("lot", s(f.lot.as_str())),
                                                ("acquired", s(f.acquired.as_str())),
                                                ("quantity", dec(&f.quantity)),
                                                ("cost", dec(&f.cost)),
                                                ("proceeds", dec(&f.proceeds)),
                                                ("expenses", dec(&f.expenses)),
                                                ("gain", dec(&f.gain)),
                                                ("taxable", Value::Bool(f.taxable)),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
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
                            ("date", s(i.date.as_str())),
                            ("kind", s(i.kind.as_str())),
                            ("quantity", dec(&i.quantity)),
                            ("value", dec(&i.value)),
                            ("section", s(i.section)),
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
                        let lines = lines(y.year);
                        dict([
                            ("year", Value::Int(y.year.into())),
                            ("disposals", Value::Int(y.disposals.into())),
                            ("taxable_proceeds", dec(&y.taxable[0])),
                            ("taxable_cost", dec(&y.taxable[1])),
                            ("taxable_expenses", dec(&y.taxable[2])),
                            ("net_gain", dec(&y.taxable[3])),
                            ("outside", dec(&y.outside)),
                            ("p23", verdict(y.p23.as_ref())),
                            ("p22_3", dec(&y.p22_3)),
                            ("p22_3_verdict", verdict(y.p22_3_verdict.as_ref())),
                            ("mining", dec(&y.mining)),
                            (
                                "lines",
                                match lines {
                                    Some(l) => dict([
                                        ("leistungen", s(l.leistungen)),
                                        ("kryptowerte", s(l.kryptowerte)),
                                        ("loss", s(l.loss)),
                                    ]),
                                    None => Value::Null,
                                },
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "holdings",
            Value::List(
                holdings
                    .iter()
                    .map(|(year, wallet, asset, quantity)| {
                        dict([
                            ("year", Value::Int((*year).into())),
                            ("wallet", s(wallet.as_str())),
                            ("asset", s(asset.as_str())),
                            ("quantity", dec(quantity)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "price_granularity",
            Value::List(
                price_granularity(conn, &profile_id)?
                    .into_iter()
                    .map(|(name, n)| {
                        dict([("granularity", s(name)), ("rows", Value::Int(n.into()))])
                    })
                    .collect(),
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn fragment(gain: &str, taxable: bool) -> Fragment {
        Fragment {
            lot: "b".into(),
            acquired: "2024-01-01".into(),
            quantity: d("0.1"),
            cost: d("100"),
            proceeds: d("100").add(&d(gain), &DECIMAL).unwrap(),
            expenses: d("0"),
            gain: d(gain),
            taxable,
        }
    }

    fn disposal(date: &str, fragments: Vec<Fragment>) -> Disposal {
        Disposal {
            transaction_id: "s".into(),
            wallet: "A".into(),
            asset: "BTC".into(),
            kind: "sell".into(),
            date: date.into(),
            fragments,
        }
    }

    #[test]
    fn freigrenzen_follow_the_year_and_unknown_years_stay_unknown() {
        let disposals = [
            // de-10: 600.00 in 2023 is not below 600.
            disposal("2023-07-01", vec![fragment("600", true)]),
            // de-08: 999.99 in 2024 is below 1,000.
            disposal(
                "2024-05-01",
                vec![fragment("999.99", true), fragment("50", false)],
            ),
            disposal("2027-05-01", vec![fragment("5000", true)]),
        ];
        let income = [Income {
            transaction_id: "i".into(),
            wallet: "A".into(),
            asset: "BTC".into(),
            date: "2024-03-01".into(),
            kind: "routing_income".into(),
            quantity: d("0.002"),
            value: d("256"),
            section: section("routing_income"),
        }];
        let years = years(&disposals, &income).unwrap();
        let json: Vec<_> = years.iter().map(|y| year_json(y).unwrap()).collect();
        assert_eq!(json[0]["p23_result"], "taxable");
        assert_eq!(json[0]["p23_freigrenze"], "600.00");
        assert_eq!(json[1]["p23_result"], "exempt_freigrenze");
        assert_eq!(json[1]["p23_amount"], "0.00");
        assert_eq!(json[1]["exempt_gain_outside_period"], "50.00");
        assert_eq!(json[1]["p22_3_result"], "taxable");
        assert_eq!(json[1]["p22_3_amount"], "256.00");
        // 2027 has no Freigrenze on file: unknown, not 2026's.
        assert_eq!(json[2]["p23_result"], "unknown");
        assert!(json[2]["p23_freigrenze"].is_null() && json[2]["p23_amount"].is_null());
        assert_eq!(section("mining"), MINING);
        assert_eq!(section("staking"), P22_3);
    }

    #[test]
    fn line_numbers_only_for_years_on_file() {
        assert_eq!(lines(2024).unwrap().kryptowerte, "41–47, 54–55");
        assert_eq!(lines(2025).unwrap().loss, "66");
        assert_eq!(lines(2023), None);
        assert_eq!(lines(2026), None);
    }
}
