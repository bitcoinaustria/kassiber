//! Ireland's country layer data: per disposal the consideration, the
//! allowable cost, the payment period and the CGT rate of its Dublin date,
//! and per calendar year the Form CG1 lines, the annual exemption, losses
//! carried forward and the tax by payment period
//! (`docs/reference/tax-country-ie.md`). The engine decides each disposal's
//! lots and gain; this module rounds, groups and places them, and maps the
//! receipts the engine quarantined to Ireland's refusal codes.

use super::*;
use kassiber_tax_countries::calendar::{self, Date};
use kassiber_tax_countries::tables;
use kassiber_tax_countries::years::Lookup;
use std::collections::BTreeMap;

/// The window around a loss in which a repurchase is flagged (s.581's four
/// weeks), in µs.
const FOUR_WEEKS_US: i64 = 28 * 86_400 * 1_000_000;

/// What a disposal is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sale,
    Swap,
    Spend,
    Gift,
    NetworkFee,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Kind::Sale => "sale",
            Kind::Swap => "swap",
            Kind::Spend => "spend",
            Kind::Gift => "gift",
            Kind::NetworkFee => "network_fee",
        }
    }
}

/// One lot a disposal took, as the engine chose it (FIFO).
#[derive(Clone, Debug)]
pub struct Lot {
    pub transaction_id: String,
    pub quantity: Decimal,
    pub cost: Decimal,
}

/// One disposal the engine computed.
#[derive(Clone, Debug)]
pub struct Disposal {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    pub kind: Kind,
    /// The instant, µs since the epoch.
    pub us: i64,
    pub quantity: Decimal,
    /// The engine's proceeds: the value the book records.
    pub proceeds: Decimal,
    /// The engine's cost: the lots' cost with their acquisition costs.
    pub cost: Decimal,
    /// Disposal costs the book records apart from the value (s.552(1));
    /// a Kassiber book nets a sale's fiat commission into its value, so
    /// they are zero there.
    pub disposal_costs: Decimal,
    pub lots: Vec<Lot>,
}

/// One Case IV receipt.
#[derive(Clone, Debug)]
pub struct Income {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    pub kind: String,
    pub us: i64,
    pub quantity: Decimal,
    pub value: Decimal,
}

/// A purchase of an asset, for the four-week review (s.581).
#[derive(Clone, Debug)]
pub struct Purchase {
    pub asset: String,
    pub us: i64,
}

/// A transaction the engine quarantined, with Ireland's code.
#[derive(Clone, Debug)]
pub struct Quarantine {
    pub transaction_id: String,
    pub code: String,
}

/// The book's Irish settings: losses brought forward from before the book
/// or from other assets (IE-R7) and the part of the €1,270 exemption still
/// available for crypto (IE-R6).
#[derive(Clone, Debug)]
pub struct Settings {
    pub losses_brought_forward_opening: Decimal,
    pub exemption_available: Option<Decimal>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            losses_brought_forward_opening: Decimal::zero(),
            exemption_available: None,
        }
    }
}

/// A disposal placed in its year and period.
#[derive(Clone, Debug)]
pub struct Row {
    pub disposal: Disposal,
    /// The Dublin date.
    pub date: Date,
    /// Disposals from 1 December pay by 31 January (`later`).
    pub later: bool,
    /// The rate of the date, or why there is none.
    pub rate: std::result::Result<Decimal, Lookup>,
    /// Half-up to cents.
    pub consideration: Decimal,
    pub allowable_cost: Decimal,
    pub gain: Decimal,
}

/// A row flagged for a reviewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    pub transaction_id: String,
    pub code: &'static str,
}

/// Whether a year's lines could be computed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Supported,
    /// A rate or the exemption is not on file for the year.
    Unknown,
    /// A disposal's date is refused with this code.
    Refused(&'static str),
}

/// One calendar year's Form CG1 lines and tax.
#[derive(Clone, Debug)]
pub struct Year {
    pub year: i32,
    pub status: Status,
    pub disposals: usize,
    pub consideration: Decimal,
    pub line_10: Decimal,
    pub line_11: Decimal,
    pub line_14: Decimal,
    pub line_17: Decimal,
    pub line_18: Decimal,
    pub line_19: Decimal,
    pub line_21: Decimal,
    pub line_22: Decimal,
    /// Per (later period, rate): the net gain taxed, initial period first,
    /// then highest rate first.
    pub rate_lines: Vec<(bool, Decimal, Decimal)>,
    pub tax_total: Decimal,
    pub payment_initial: Decimal,
    pub payment_later: Decimal,
    pub overpaid_initial: Decimal,
    pub case_iv_income: Decimal,
    /// The exemption available that year.
    pub exemption: Option<Decimal>,
}

fn ctx_add(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.add(b, &DECIMAL).map_err(arithmetic)
}
fn ctx_sub(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.sub(b, &DECIMAL).map_err(arithmetic)
}
fn ctx_mul(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.mul(b, &DECIMAL).map_err(arithmetic)
}
fn min(a: &Decimal, b: &Decimal) -> Decimal {
    if a <= b {
        a.clone()
    } else {
        b.clone()
    }
}
fn max_zero(a: Decimal) -> Decimal {
    if a.is_negative() {
        Decimal::zero()
    } else {
        a
    }
}

/// Half-up to cents, as Irish figures are shown (IE-R15).
pub fn cents(value: &Decimal) -> Result<Decimal> {
    value
        .to_fixed_places(2, Rounding::HalfUp)
        .map_err(arithmetic)?
        .parse()
        .map_err(|_| AppError::new("internal_error", "invalid cents"))
}

/// A cent amount's text (`1234.50`).
pub fn money(value: &Decimal) -> Result<String> {
    value
        .to_fixed_places(2, Rounding::HalfUp)
        .map_err(arithmetic)
}

/// The Dublin date of an instant; Kassiber has no Dublin calendar before
/// 1996, so an older instant is refused rather than given a guessed offset.
pub fn dublin_date(us: i64) -> Result<Date> {
    calendar::zone(kassiber_tax_countries::DUBLIN)
        .expect("the registry has Dublin's calendar")
        .local_date(us)
        .map_err(|error| AppError::new("validation", error.to_string()))
}

fn rate_on(date: Date) -> Result<std::result::Result<Decimal, Lookup>> {
    crate::tax_years::table_value(&tables::IE_CGT_RATE, date)
}

/// Each disposal's row and the rows' flags, in disposal order.
pub fn place(disposals: &[Disposal], purchases: &[Purchase]) -> Result<(Vec<Row>, Vec<Flag>)> {
    let mut rows = Vec::new();
    let mut flags = Vec::new();
    for disposal in disposals {
        let date = dublin_date(disposal.us)?;
        let rate = rate_on(date)?;
        let consideration = cents(&ctx_add(&disposal.proceeds, &disposal.disposal_costs)?)?;
        let allowable_cost = cents(&ctx_add(&disposal.cost, &disposal.disposal_costs)?)?;
        let gain = ctx_sub(&consideration, &allowable_cost)?;
        let flag = |code| Flag {
            transaction_id: disposal.transaction_id.clone(),
            code,
        };
        if disposal.kind == Kind::Gift {
            flags.push(flag("ie_gift_connected_party"));
        }
        if matches!(&rate, Ok(r) if r == &"0.31".parse::<Decimal>().unwrap()) {
            flags.push(flag("ie_rate_31_pending_finance_act"));
        }
        if gain.is_negative()
            && disposal.kind == Kind::Sale
            && purchases.iter().any(|purchase| {
                purchase.asset == disposal.asset
                    && purchase.us != disposal.us
                    && (purchase.us - disposal.us).abs() <= FOUR_WEEKS_US
            })
        {
            flags.push(flag("ie_s581_four_week_review"));
        }
        rows.push(Row {
            disposal: disposal.clone(),
            later: (date.month, date.day) > (11, 30),
            date,
            rate,
            consideration,
            allowable_cost,
            gain,
        });
    }
    Ok((rows, flags))
}

/// Sets `deductions` against the positive buckets, highest rate first and
/// the initial period before the later one; what remains of each.
fn allocate(
    buckets: &BTreeMap<(bool, Decimal), Decimal>,
    deductions: &Decimal,
) -> Result<Vec<(bool, Decimal, Decimal)>> {
    let mut order: Vec<(bool, Decimal)> = buckets.keys().cloned().collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut left = deductions.clone();
    let mut remaining = Vec::new();
    for key in order {
        let gain = &buckets[&key];
        let used = min(gain, &left);
        left = ctx_sub(&left, &used)?;
        remaining.push((key.0, key.1, ctx_sub(gain, &used)?));
    }
    Ok(remaining)
}

/// The tax on allocated net gains, half-up to cents.
fn tax(gains: &[(bool, Decimal, Decimal)]) -> Result<Decimal> {
    let mut total = Decimal::zero();
    for (_, rate, amount) in gains {
        total = ctx_add(&total, &ctx_mul(amount, rate)?)?;
    }
    cents(&total)
}

/// The gains, losses, and the net after losses brought forward and the
/// exemption: lines 10, 11, 14, 17, 18, 19.
fn net_lines(rows: &[&Row], forward: &Decimal, exemption: &Decimal) -> Result<[Decimal; 6]> {
    let mut gains = Decimal::zero();
    let mut losses = Decimal::zero();
    for row in rows {
        if row.gain.is_positive() {
            gains = ctx_add(&gains, &row.gain)?;
        } else if row.gain.is_negative() {
            losses = ctx_sub(&losses, &row.gain)?;
        }
    }
    let line_14 = max_zero(ctx_sub(&gains, &losses)?);
    let line_17 = min(forward, &line_14);
    let line_18 = min(exemption, &ctx_sub(&line_14, &line_17)?);
    let line_19 = ctx_sub(&ctx_sub(&line_14, &line_17)?, &line_18)?;
    Ok([gains, losses, line_14, line_17, line_18, line_19])
}

/// Every year from the first to the last with a disposal or income, losses
/// carrying forward between them (IE-R6 to IE-R8).
pub fn years(rows: &[Row], income: &[Income], settings: &Settings) -> Result<Vec<Year>> {
    let mut by_year: BTreeMap<i32, Vec<&Row>> = BTreeMap::new();
    for row in rows {
        by_year.entry(row.date.year).or_default().push(row);
    }
    let mut case_iv: BTreeMap<i32, Decimal> = BTreeMap::new();
    for item in income {
        let year = dublin_date(item.us)?.year;
        let total = case_iv.entry(year).or_insert_with(Decimal::zero);
        *total = ctx_add(total, &cents(&item.value)?)?;
    }
    let first = by_year.keys().chain(case_iv.keys()).min().copied();
    let last = by_year.keys().chain(case_iv.keys()).max().copied();
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(Vec::new());
    };
    let mut forward = settings.losses_brought_forward_opening.clone();
    let mut out = Vec::new();
    for year in first..=last {
        let rows: Vec<&Row> = by_year.get(&year).cloned().unwrap_or_default();
        let mut consideration = Decimal::zero();
        for row in &rows {
            consideration = ctx_add(&consideration, &row.consideration)?;
        }
        let exemption = match crate::tax_years::table_value(
            &tables::IE_ANNUAL_EXEMPTION,
            Date::new(year, 1, 1).expect("a date"),
        )? {
            Ok(full) => Some(match &settings.exemption_available {
                Some(available) => min(available, &full),
                None => full,
            }),
            Err(_) => None,
        };
        let mut status = Status::Supported;
        for row in &rows {
            match &row.rate {
                Err(Lookup::Refused(code)) => status = Status::Refused(code),
                Err(_) if status == Status::Supported => status = Status::Unknown,
                _ => {}
            }
        }
        if exemption.is_none() && status == Status::Supported {
            status = Status::Unknown;
        }
        let zero = Decimal::zero();
        let [line_10, line_11, line_14, line_17, line_18, line_19] =
            net_lines(&rows, &forward, exemption.as_ref().unwrap_or(&zero))?;
        let line_21 = max_zero(ctx_sub(&line_11, &line_10)?);
        let line_22 = ctx_sub(&forward, &line_17)?;
        let mut item = Year {
            year,
            status: status.clone(),
            disposals: rows.len(),
            consideration,
            line_10,
            line_11: line_11.clone(),
            line_14,
            line_17: line_17.clone(),
            line_18: line_18.clone(),
            line_19,
            line_21: line_21.clone(),
            line_22: line_22.clone(),
            rate_lines: Vec::new(),
            tax_total: Decimal::zero(),
            payment_initial: Decimal::zero(),
            payment_later: Decimal::zero(),
            overpaid_initial: Decimal::zero(),
            case_iv_income: case_iv.get(&year).cloned().unwrap_or_else(Decimal::zero),
            exemption: exemption.clone(),
        };
        if status == Status::Supported {
            let exemption = exemption.expect("a supported year has its exemption");
            let mut buckets: BTreeMap<(bool, Decimal), Decimal> = BTreeMap::new();
            for row in &rows {
                if row.gain.is_positive() {
                    let rate = row.rate.clone().expect("a supported year has its rates");
                    let slot = buckets
                        .entry((row.later, rate))
                        .or_insert_with(Decimal::zero);
                    *slot = ctx_add(slot, &row.gain)?;
                }
            }
            let deductions = ctx_add(&ctx_add(&line_11, &line_17)?, &line_18)?;
            let annual = allocate(&buckets, &deductions)?;
            let tax_total = tax(&annual)?;
            // The initial period as if the year ended on 30 November (s.958).
            let initial_rows: Vec<&Row> = rows.iter().copied().filter(|r| !r.later).collect();
            let [_, losses_i, _, line_17_i, line_18_i, _] =
                net_lines(&initial_rows, &forward, &exemption)?;
            let initial_buckets: BTreeMap<(bool, Decimal), Decimal> = buckets
                .iter()
                .filter(|((later, _), _)| !later)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let deductions = ctx_add(&ctx_add(&losses_i, &line_17_i)?, &line_18_i)?;
            let tax_initial = tax(&allocate(&initial_buckets, &deductions)?)?;
            let later = ctx_sub(&tax_total, &tax_initial)?;
            let mut lines = annual;
            lines.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
            item.rate_lines = lines;
            item.tax_total = tax_total;
            item.payment_initial = tax_initial;
            item.payment_later = max_zero(later.clone());
            item.overpaid_initial = max_zero(later.neg(&DECIMAL).map_err(arithmetic)?);
        }
        out.push(item);
        forward = ctx_add(&line_21, &line_22)?;
    }
    Ok(out)
}

/// A rate as its percentage text (`33`).
pub fn percent(rate: &Decimal) -> Result<String> {
    let pct = ctx_mul(rate, &Decimal::from(100u8))?;
    pct.to_fixed_places(0, Rounding::HalfEven)
        .map_err(arithmetic)
}

/// The Form CG1 line of a rate line: 24 for the initial period, 25 for the
/// later one.
pub fn rate_line(later: bool, rate: &Decimal) -> Result<String> {
    Ok(format!(
        "line_{}_at_{}pct",
        if later { 25 } else { 24 },
        percent(rate)?
    ))
}

/// A quantity as the vectors write it: at most 11 places, no trailing zeros.
pub fn quantity_text(d: &Decimal) -> Result<String> {
    let text = d
        .to_fixed_places(11, Rounding::HalfEven)
        .map_err(arithmetic)?;
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    Ok(if text == "-0" { "0".into() } else { text })
}

/// The figures in the shared vector schema
/// (`docs/reference/tax-country-ie-vectors.json`).
pub fn vector_json(
    rows: &[Row],
    flags: &[Flag],
    income: &[Income],
    quarantines: &[Quarantine],
    years: &[Year],
) -> Result<serde_json::Value> {
    use serde_json::json;
    let mut disposals = Vec::new();
    for row in rows {
        let d = &row.disposal;
        let mut lots = Vec::new();
        for lot in &d.lots {
            lots.push(json!({"lot": lot.transaction_id, "qty": quantity_text(&lot.quantity)?, "cost": money(&lot.cost)?}));
        }
        disposals.push(json!({
            "event": d.transaction_id, "asset": d.asset, "qty": quantity_text(&d.quantity)?,
            "kind": d.kind.code(), "date": row.date.iso(),
            "period": if row.later { "later" } else { "initial" },
            "rate": match &row.rate { Ok(rate) => json!(rate.to_string()), Err(other) => json!(format!("{other:?}")) },
            "consideration": money(&row.consideration)?, "allowable_cost": money(&row.allowable_cost)?,
            "gain": money(&row.gain)?, "lots": lots,
        }));
    }
    let mut years_out = serde_json::Map::new();
    for year in years {
        let mut rate_lines = serde_json::Map::new();
        for (later, rate, amount) in &year.rate_lines {
            rate_lines.insert(rate_line(*later, rate)?, json!(money(amount)?));
        }
        let mut block = json!({
            "line_1k_disposals": year.disposals, "line_1k_consideration": money(&year.consideration)?,
            "line_10": money(&year.line_10)?, "line_11": money(&year.line_11)?,
            "line_14": money(&year.line_14)?, "line_17": money(&year.line_17)?,
            "line_18": money(&year.line_18)?, "line_19": money(&year.line_19)?,
            "line_21": money(&year.line_21)?, "line_22": money(&year.line_22)?,
            "line_23": money(&ctx_add(&year.line_21, &year.line_22)?)?,
            "rate_lines": rate_lines, "tax_total": money(&year.tax_total)?,
            "payment_initial": money(&year.payment_initial)?, "payment_later": money(&year.payment_later)?,
            "overpaid_initial": money(&year.overpaid_initial)?, "case_iv_income": money(&year.case_iv_income)?,
        });
        match &year.status {
            Status::Supported => {}
            Status::Unknown => block["status"] = json!("unknown"),
            Status::Refused(code) => block["status"] = json!(code),
        }
        years_out.insert(year.year.to_string(), block);
    }
    Ok(json!({
        "disposals": disposals,
        "income": income.iter().map(|i| json!({"event": i.transaction_id, "asset": i.asset, "kind": i.kind,
                                                "schedule": "case_iv", "value": money(&i.value).unwrap_or_default()}))
            .collect::<Vec<_>>(),
        "quarantines": quarantines.iter().map(|q| json!({"event": q.transaction_id, "reason": q.code})).collect::<Vec<_>>(),
        "flags": flags.iter().map(|f| json!({"event": f.transaction_id, "code": f.code})).collect::<Vec<_>>(),
        "years": years_out,
    }))
}

/// Ireland's code for a quarantined receipt or disposal: the code a
/// refused receipt carries (IE-R9), lost or stolen coins whatever the
/// engine held them for (an unpriced loss too, IE-R13); any other
/// quarantine keeps the engine's reason.
pub fn quarantine_code(reason: &str, kind: &str, code: Option<&str>) -> String {
    if let Some(code) = code {
        return code.into();
    }
    if ["lost", "stolen", "theft"].iter().any(|t| kind.contains(t)) {
        return "ie_lost_requires_negligible_value_claim".into();
    }
    reason.into()
}

/// A book's disposals, income, purchases and quarantines, as [`load`] reads them.
pub type Book = (Vec<Disposal>, Vec<Income>, Vec<Purchase>, Vec<Quarantine>);

/// The book's disposals, income, purchases and quarantines, from its journal
/// and its quarantine list. A `wallet` id limits them to that wallet.
pub fn load(conn: &Connection, profile_id: &str, wallet: Option<&str>) -> Result<Book> {
    let entries = rows(
        conn,
        "SELECT je.transaction_id,je.occurred_at,je.entry_type,je.asset,je.quantity,\
         je.proceeds,je.proceeds_exact,je.cost_basis,je.cost_basis_exact,\
         je.calculation_json,je.calculation_compact,COALESCE(w.label,'') AS wallet,\
         COALESCE(NULLIF(t.kind_override,''),t.kind,'') AS kind \
         FROM journal_entries je LEFT JOIN wallets w ON w.id=je.wallet_id \
         LEFT JOIN transactions t ON t.id=je.transaction_id \
         WHERE je.profile_id=?1 AND (?2 IS NULL OR je.wallet_id=?2) \
         AND je.entry_type IN ('acquisition','disposal','income','fee','transfer_fee') \
         ORDER BY je.occurred_at,je.created_at,je.id",
        kassiber_store::rusqlite::params![profile_id, wallet],
    )?;
    let income_rows: std::collections::HashSet<String> = entries
        .iter()
        .filter(|e| text(e, "entry_type") == "income")
        .map(|e| text(e, "transaction_id"))
        .collect();
    let (mut disposals, mut income, mut purchases) = (Vec::new(), Vec::new(), Vec::new());
    for entry in &entries {
        let us = crate::layer_reads::instant(&text(entry, "occurred_at"))?;
        let transaction_id = text(entry, "transaction_id");
        let asset = text(entry, "asset");
        let kind = kassiber_pytext::lower(kassiber_pytext::strip(&text(entry, "kind")));
        let quantity = btc(int(entry, "quantity"))?.copy_abs();
        match text(entry, "entry_type").as_str() {
            "acquisition" => {
                if !income_rows.contains(&transaction_id) {
                    purchases.push(Purchase { asset, us });
                }
            }
            "income" => income.push(Income {
                transaction_id,
                wallet: text(entry, "wallet"),
                asset,
                kind,
                us,
                quantity,
                value: crate::layer_reads::exact(entry, "proceeds")?,
            }),
            entry_type => {
                let mut lots = Vec::new();
                if let Some(calculation) = crate::gain_explanation::calculation(entry) {
                    if let Value::List(fragments) = value(&calculation, "fragments") {
                        for fragment in fragments {
                            let lot = value(fragment, "lot");
                            if matches!(lot, Value::Null) {
                                continue;
                            }
                            lots.push(Lot {
                                transaction_id: text(lot, "transaction_id"),
                                quantity: btc(int(fragment, "quantity_msat"))?.copy_abs(),
                                cost: decimal(value(fragment, "cost_basis_exact"))?,
                            });
                        }
                    }
                }
                let kind = if matches!(entry_type, "fee" | "transfer_fee") {
                    Kind::NetworkFee
                } else if kind.contains("gift") {
                    Kind::Gift
                } else if kind == "spend" || kind == "payment" {
                    Kind::Spend
                } else if kind == "swap" {
                    Kind::Swap
                } else {
                    Kind::Sale
                };
                disposals.push(Disposal {
                    transaction_id,
                    wallet: text(entry, "wallet"),
                    asset,
                    kind,
                    us,
                    quantity,
                    proceeds: crate::layer_reads::exact(entry, "proceeds")?,
                    cost: crate::layer_reads::exact(entry, "cost_basis")?,
                    disposal_costs: Decimal::zero(),
                    lots,
                });
            }
        }
    }
    let quarantines = crate::layer_reads::quarantines(conn, profile_id, wallet)?
        .into_iter()
        .map(|held| Quarantine {
            code: quarantine_code(&held.reason, &held.kind, held.code.as_deref()),
            transaction_id: held.transaction_id,
        })
        .collect();
    Ok((disposals, income, purchases, quarantines))
}

fn dec(d: &Decimal) -> Value {
    s(d.to_string())
}

/// The layer's part of a report model: rows, flags, income, quarantines and
/// years, exact and unformatted.
pub fn model(conn: &Connection, profile: &Value, wallet: Option<&str>) -> Result<Value> {
    let (disposals, income, purchases, quarantines) = load(conn, &text(profile, "id"), wallet)?;
    let (rows, flags) = place(&disposals, &purchases)?;
    let years = years(&rows, &income, &Settings::default())?;
    let row = |r: &Row| {
        let d = &r.disposal;
        dict([
            ("transaction_id", s(d.transaction_id.as_str())),
            ("wallet", s(d.wallet.as_str())),
            ("asset", s(d.asset.as_str())),
            ("kind", s(d.kind.code())),
            ("date", s(r.date.iso())),
            ("period", s(if r.later { "later" } else { "initial" })),
            (
                "rate",
                match &r.rate {
                    Ok(rate) => dec(rate),
                    Err(_) => Value::Null,
                },
            ),
            ("quantity", dec(&d.quantity)),
            ("consideration", dec(&r.consideration)),
            ("allowable_cost", dec(&r.allowable_cost)),
            ("gain", dec(&r.gain)),
        ])
    };
    let mut year_values = Vec::new();
    for y in &years {
        let mut lines = Vec::new();
        for (later, rate, amount) in &y.rate_lines {
            lines.push(dict([
                ("line", s(if *later { "25" } else { "24" })),
                ("rate", dec(rate)),
                ("amount", dec(amount)),
            ]));
        }
        year_values.push(dict([
            ("year", Value::Int(y.year.into())),
            (
                "status",
                s(match &y.status {
                    Status::Supported => "supported",
                    Status::Unknown => "unknown",
                    Status::Refused(code) => code,
                }),
            ),
            ("disposals", Value::Int(y.disposals.into())),
            ("consideration", dec(&y.consideration)),
            ("line_10", dec(&y.line_10)),
            ("line_11", dec(&y.line_11)),
            ("line_14", dec(&y.line_14)),
            ("line_17", dec(&y.line_17)),
            ("line_18", dec(&y.line_18)),
            ("line_19", dec(&y.line_19)),
            ("line_21", dec(&y.line_21)),
            ("line_22", dec(&y.line_22)),
            ("line_23", dec(&ctx_add(&y.line_21, &y.line_22)?)),
            ("rate_lines", Value::List(lines)),
            ("tax_total", dec(&y.tax_total)),
            ("payment_initial", dec(&y.payment_initial)),
            ("payment_later", dec(&y.payment_later)),
            ("overpaid_initial", dec(&y.overpaid_initial)),
            ("case_iv_income", dec(&y.case_iv_income)),
            ("exemption", y.exemption.as_ref().map_or(Value::Null, dec)),
        ]));
    }
    Ok(dict([
        ("country", s("ie")),
        ("wallet_scoped", Value::Bool(wallet.is_some())),
        ("rows", Value::List(rows.iter().map(row).collect())),
        (
            "flags",
            Value::List(
                flags
                    .iter()
                    .map(|f| {
                        dict([
                            ("transaction_id", s(f.transaction_id.as_str())),
                            ("code", s(f.code)),
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
                        Ok(dict([
                            ("transaction_id", s(i.transaction_id.as_str())),
                            ("wallet", s(i.wallet.as_str())),
                            ("asset", s(i.asset.as_str())),
                            ("date", s(dublin_date(i.us)?.iso())),
                            ("kind", s(i.kind.as_str())),
                            ("quantity", dec(&i.quantity)),
                            ("value", dec(&i.value)),
                        ]))
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        ),
        (
            "quarantines",
            Value::List(
                quarantines
                    .iter()
                    .map(|q| {
                        dict([
                            ("transaction_id", s(q.transaction_id.as_str())),
                            ("code", s(q.code.as_str())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("years", Value::List(year_values)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn quarantines_take_irelands_codes() {
        let code = |reason, kind, code| quarantine_code(reason, kind, code);
        assert_eq!(
            code(
                "acquisition_valuation_unsupported",
                "airdrop",
                Some("ie_airdrop_requires_review")
            ),
            "ie_airdrop_requires_review"
        );
        assert_eq!(
            code("non_sale_disposal_kind", "lost", None),
            "ie_lost_requires_negligible_value_claim"
        );
        // ie-17: an unpriced loss is held for its price first.
        assert_eq!(
            code("missing_spot_price", "stolen", None),
            "ie_lost_requires_negligible_value_claim"
        );
        assert_eq!(
            code("non_sale_disposal_kind", "donation", None),
            "non_sale_disposal_kind"
        );
        assert_eq!(code("insufficient_lots", "sell", None), "insufficient_lots");
    }

    #[test]
    fn cents_round_half_up_and_rate_lines_name_their_period() {
        // ie-10: 0.825 is 0.83.
        assert_eq!(money(&d("0.825")).unwrap(), "0.83");
        assert_eq!(rate_line(false, &d("0.33")).unwrap(), "line_24_at_33pct");
        assert_eq!(rate_line(true, &d("0.31")).unwrap(), "line_25_at_31pct");
    }
}
