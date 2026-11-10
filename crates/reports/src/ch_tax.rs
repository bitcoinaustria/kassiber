//! The Swiss country layer's data (`docs/reference/tax-country-ch.md`): a
//! private investor's capital gains are tax-free, so the layer reports the
//! income from crypto by category, the wealth at 31 December for the
//! cantonal wealth tax, the KS 36 safe-harbour indicators, and the realized
//! gains for information. Proceeds, costs, gains and income values are the
//! tax engine's; dates are Zurich dates; nothing here guesses a value: a
//! holding without a year-end value leaves the year's wealth unknown, and a
//! criterion without its input is `not_assessable`, never met.

use super::*;
use kassiber_tax_countries::calendar::Date;
use std::collections::BTreeMap;

/// Income from movable assets (Art. 20 Abs. 1 DBG).
pub const MOVABLE: &str = "movable_assets_income";
/// Income under the general clause (Art. 16 Abs. 1 DBG).
pub const GENERAL: &str = "general_clause_income";
/// Mining rewards, possibly self-employment income.
pub const MINING: &str = "mining_income";

/// The income category of a receipt of `kind` (CH-INC-*).
pub fn category(kind: &str) -> &'static str {
    match kind {
        "staking" | "lending_interest" | "interest" | "airdrop" => MOVABLE,
        "mining" | "mining_reward" => MINING,
        // routing income, a classified service airdrop, other income.
        _ => GENERAL,
    }
}

/// The ESTV Kursliste's source tag for a year-end value (`rates set ...
/// --source estv_kursliste`).
pub const ESTV: &str = "estv_kursliste";
/// A year-end value from any other rate source: the market close.
pub const MARKET: &str = "market_close";

/// One consumed lot of a disposal, for information.
#[derive(Clone, Debug)]
pub struct Fragment {
    pub lot: String,
    /// The acquisition's Zurich date, or empty when unknown.
    pub acquired: String,
    pub quantity: Decimal,
    pub cost: Decimal,
    pub proceeds: Decimal,
    pub gain: Decimal,
    /// KS 36 criterion 1: held at least six calendar months.
    pub held_6_months: bool,
}

#[derive(Clone, Debug)]
pub struct Disposal {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    pub kind: String,
    /// The Zurich date.
    pub date: String,
    pub fragments: Vec<Fragment>,
}

#[derive(Clone, Debug)]
pub struct Income {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    pub date: String,
    pub kind: String,
    pub quantity: Decimal,
    pub value: Decimal,
    pub category: &'static str,
}

/// A receipt listed but kept out of the income total: salary (on the
/// Lohnausweis) or a gift.
#[derive(Clone, Debug)]
pub struct Outside {
    pub transaction_id: String,
    pub date: String,
    /// `wages` or `gift_in`.
    pub kind: &'static str,
    pub value: Decimal,
}

impl Outside {
    pub fn treatment(&self) -> &'static str {
        if self.kind == "wages" {
            "employment income on the Lohnausweis (Art. 17 DBG)"
        } else {
            "gift: not income (Art. 24 lit. a DBG); cantonal gift tax"
        }
    }
}

/// A change of a wallet's holding of an asset on a Zurich date.
#[derive(Clone, Debug)]
pub struct Movement {
    pub date: String,
    pub wallet: String,
    pub asset: String,
    pub quantity: Decimal,
}

/// A year-end value per unit and where it came from.
#[derive(Clone, Debug)]
pub struct Valuation {
    pub unit: Decimal,
    pub source: String,
}

/// The user's KS 36 inputs and declarations.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// The period's net income, by tax year (criterion 3).
    pub net_income: BTreeMap<i32, Decimal>,
    /// Debt-financed without cover by taxable investment income (C4).
    pub debt_financed_without_cover: Option<bool>,
    /// Derivatives only to hedge own positions (C5).
    pub derivatives_only_hedging: Option<bool>,
    /// The book is a professional trader's (business assets): refused.
    pub professional_trader: bool,
    /// The canton, informational.
    pub canton: Option<String>,
}

/// What a book holds for the layer.
#[derive(Clone, Debug, Default)]
pub struct Book {
    pub disposals: Vec<Disposal>,
    pub income: Vec<Income>,
    pub outside: Vec<Outside>,
    /// Purchases (acquisitions other than income, salary and gifts): their
    /// values count towards criterion 2's volume.
    pub purchases: Vec<(String, Decimal)>,
    pub movements: Vec<Movement>,
}

/// One holding at 31 December.
#[derive(Clone, Debug)]
pub struct WealthRow {
    pub wallet: String,
    pub asset: String,
    pub quantity: Decimal,
    pub valuation: Option<Valuation>,
    pub value: Option<Decimal>,
    pub flags: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Wealth {
    pub rows: Vec<WealthRow>,
    /// `None` when a holding has no value.
    pub total: Option<Decimal>,
}

/// A KS 36 criterion's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Criterion {
    Met,
    NotMet,
    NotAssessable,
    /// Criterion 1 in a year without disposals.
    NoDisposals,
}

impl Criterion {
    pub fn code(self) -> &'static str {
        match self {
            Criterion::Met => "met",
            Criterion::NotMet => "not_met",
            Criterion::NotAssessable => "not_assessable",
            Criterion::NoDisposals => "no_disposals",
        }
    }
}

/// The KS 36 indicators of a year.
#[derive(Clone, Debug)]
pub struct Ks36 {
    pub c1: Criterion,
    pub c2: Criterion,
    pub volume: Decimal,
    pub limit: Option<Decimal>,
    pub c3: Criterion,
    pub gains: Decimal,
    pub half_income: Option<Decimal>,
    pub c4: Criterion,
    pub c5: Criterion,
    /// `met`, `not_met` or `inconclusive`.
    pub overall: &'static str,
}

#[derive(Clone, Debug)]
pub struct Year {
    pub year: i32,
    pub realized_gain: Decimal,
    pub income_by_category: BTreeMap<&'static str, Decimal>,
    pub income_total: Decimal,
    pub outside: Vec<Outside>,
    pub wealth: Wealth,
    pub ks36: Ks36,
}

fn add(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.add(b, &DECIMAL).map_err(arithmetic)
}

fn year_of(date: &str) -> i32 {
    date.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0)
}

/// KS 36 criterion 1 for one lot: held at least six calendar months,
/// month ends clamped (bought 31 August, 28 February counts).
pub fn held_6_months(acquired: &str, sold: &str) -> bool {
    match (Date::parse(acquired), Date::parse(sold)) {
        (Some(acquired), Some(sold)) => sold >= acquired.add_months(6),
        // Without an acquisition date the criterion is not shown as met.
        _ => false,
    }
}

/// Each wallet's holding of each asset at the end of 31 December of
/// `year`, valued by `value` (per year and asset): L-BTC without its own
/// value at Bitcoin's, flagged (CH-WEALTH, Q4); a holding without any value
/// leaves the total unknown (CH-WEALTH-FALLBACK, never cost).
pub fn wealth(
    movements: &[Movement],
    year: i32,
    value: &dyn Fn(i32, &str) -> Option<Valuation>,
) -> Result<Wealth> {
    let mut holdings: BTreeMap<(String, String), Decimal> = BTreeMap::new();
    for movement in movements.iter().filter(|m| year_of(&m.date) <= year) {
        let slot = holdings
            .entry((movement.wallet.clone(), movement.asset.clone()))
            .or_insert_with(Decimal::zero);
        *slot = add(slot, &movement.quantity)?;
    }
    let mut rows = Vec::new();
    let mut total = Some(Decimal::zero());
    for ((wallet, asset), quantity) in holdings {
        if !quantity.is_positive() {
            continue;
        }
        let mut flags = Vec::new();
        let mut valuation = value(year, &asset);
        if valuation.is_none() && asset == "LBTC" {
            valuation = value(year, "BTC");
            if valuation.is_some() {
                flags.push("lbtc_valued_as_btc");
            }
        }
        let amount = match &valuation {
            Some(valuation) => {
                if valuation.source != ESTV {
                    flags.push("not_estv_tax_value");
                }
                let amount = quantity
                    .mul(&valuation.unit, &DECIMAL)
                    .map_err(arithmetic)?;
                total = match total {
                    Some(total) => Some(add(&total, &amount)?),
                    None => None,
                };
                Some(amount)
            }
            None => {
                flags.push("missing_valuation");
                total = None;
                None
            }
        };
        rows.push(WealthRow {
            wallet,
            asset,
            quantity,
            valuation,
            value: amount,
            flags,
        });
    }
    Ok(Wealth { rows, total })
}

/// The figures of `years` (every year with an event, and any the caller
/// adds), in year order.
pub fn years(
    book: &Book,
    extra: &[i32],
    value: &dyn Fn(i32, &str) -> Option<Valuation>,
    inputs: &Inputs,
) -> Result<Vec<Year>> {
    let mut all: Vec<i32> = book
        .disposals
        .iter()
        .map(|d| year_of(&d.date))
        .chain(book.income.iter().map(|i| year_of(&i.date)))
        .chain(book.outside.iter().map(|o| year_of(&o.date)))
        .chain(book.purchases.iter().map(|(date, _)| year_of(date)))
        .chain(book.movements.iter().map(|m| year_of(&m.date)))
        .chain(extra.iter().copied())
        .collect();
    all.sort_unstable();
    all.dedup();
    let five = Decimal::from(5u8);
    let two = Decimal::from(2u8);
    let mut out = Vec::new();
    for year in all {
        let fragments: Vec<&Fragment> = book
            .disposals
            .iter()
            .filter(|d| year_of(&d.date) == year)
            .flat_map(|d| &d.fragments)
            .collect();
        let mut gains = Decimal::zero();
        for fragment in &fragments {
            gains = add(&gains, &fragment.gain)?;
        }
        let mut income_by_category = BTreeMap::new();
        let mut income_total = Decimal::zero();
        for item in book.income.iter().filter(|i| year_of(&i.date) == year) {
            let slot = income_by_category
                .entry(item.category)
                .or_insert_with(Decimal::zero);
            *slot = add(slot, &item.value)?;
            income_total = add(&income_total, &item.value)?;
        }
        // Criterion 2's volume: purchases and sale proceeds; a swap's
        // received leg is a purchase and its given leg a sale.
        let mut volume = Decimal::zero();
        for (date, value) in &book.purchases {
            if year_of(date) == year {
                volume = add(&volume, value)?;
            }
        }
        for fragment in &fragments {
            volume = add(&volume, &fragment.proceeds)?;
        }
        let c1 = if fragments.is_empty() {
            Criterion::NoDisposals
        } else if fragments.iter().all(|f| f.held_6_months) {
            Criterion::Met
        } else {
            Criterion::NotMet
        };
        // Kassiber knows only the crypto part of the start-of-year holdings
        // (basis `crypto_only`, Q1).
        let start = wealth(&book.movements, year - 1, value)?;
        let limit = match start.total {
            Some(total) if !total.is_zero() => {
                Some(total.mul(&five, &DECIMAL).map_err(arithmetic)?)
            }
            _ => None,
        };
        let c2 = match &limit {
            None => Criterion::NotAssessable,
            Some(limit) if &volume <= limit => Criterion::Met,
            Some(_) => Criterion::NotMet,
        };
        let half_income = match inputs.net_income.get(&year) {
            Some(income) => Some(income.div(&two, &DECIMAL).map_err(arithmetic)?),
            None => None,
        };
        let c3 = match &half_income {
            None => Criterion::NotAssessable,
            Some(half) if &gains < half => Criterion::Met,
            Some(_) => Criterion::NotMet,
        };
        let c4 = match inputs.debt_financed_without_cover {
            None => Criterion::NotAssessable,
            Some(true) => Criterion::NotMet,
            Some(false) => Criterion::Met,
        };
        let c5 = match inputs.derivatives_only_hedging {
            None => Criterion::NotAssessable,
            Some(true) => Criterion::Met,
            Some(false) => Criterion::NotMet,
        };
        let results = [c1, c2, c3, c4, c5];
        let overall = if results.contains(&Criterion::NotMet) {
            "not_met"
        } else if results.contains(&Criterion::NotAssessable) {
            "inconclusive"
        } else {
            "met"
        };
        out.push(Year {
            year,
            realized_gain: gains.clone(),
            income_by_category,
            income_total,
            outside: book
                .outside
                .iter()
                .filter(|o| year_of(&o.date) == year)
                .cloned()
                .collect(),
            wealth: wealth(&book.movements, year, value)?,
            ks36: Ks36 {
                c1,
                c2,
                volume,
                limit,
                c3,
                gains,
                half_income,
                c4,
                c5,
                overall,
            },
        });
    }
    Ok(out)
}

/// A fiat amount at centimes, half-even.
pub fn cents(value: &Decimal) -> Result<String> {
    value
        .to_fixed_places(2, Rounding::HalfEven)
        .map_err(arithmetic)
}

/// A quantity as the vectors render it: at most 11 places, no trailing
/// zeros.
fn quantity_text(d: &Decimal) -> Result<String> {
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

/// One year's figures in the shared vector schema's `years` form
/// (`docs/reference/tax-country-ch-vectors.json`).
pub fn year_json(year: &Year) -> Result<serde_json::Value> {
    use serde_json::{json, Value as Json};
    let money = |d: &Decimal| cents(d).map(Json::from);
    let maybe = |d: &Option<Decimal>| -> Result<Json> { d.as_ref().map_or(Ok(Json::Null), money) };
    let mut by_category = serde_json::Map::new();
    for (category, total) in &year.income_by_category {
        by_category.insert((*category).to_owned(), money(total)?);
    }
    let mut rows = Vec::new();
    for row in &year.wealth.rows {
        rows.push(json!({
            "wallet": row.wallet,
            "asset": row.asset,
            "quantity": quantity_text(&row.quantity)?,
            "unit_value_chf": match &row.valuation { Some(v) => money(&v.unit)?, None => Json::Null },
            "value_chf": maybe(&row.value)?,
            "source": row.valuation.as_ref().map(|v| v.source.clone()),
            "flags": row.flags,
        }));
    }
    let outside: Vec<Json> = year
        .outside
        .iter()
        .map(|o| {
            Ok(
                json!({"event": o.transaction_id, "date": o.date, "kind": o.kind,
                      "value_chf": money(&o.value)?, "treatment": o.treatment()}),
            )
        })
        .collect::<Result<_>>()?;
    let ks = &year.ks36;
    Ok(json!({
        "taxable_capital_gains_chf": "0.00",
        "informational_realized_gain_chf": money(&year.realized_gain)?,
        "income_by_category_chf": by_category,
        "income_total_chf": money(&year.income_total)?,
        "not_in_income_total": outside,
        "wealth_31_dec": {"rows": rows, "total_chf": maybe(&year.wealth.total)?,
                          "complete": year.wealth.total.is_some()},
        "ks36": {
            "c1_holding_6_months": ks.c1.code(),
            "c2_volume_5x": {"result": ks.c2.code(), "volume_chf": money(&ks.volume)?,
                             "limit_chf": maybe(&ks.limit)?, "basis": "crypto_only"},
            "c3_gains_under_half_income": {"result": ks.c3.code(), "realized_gains_chf": money(&ks.gains)?,
                                           "half_net_income_chf": maybe(&ks.half_income)?},
            "c4_financing": ks.c4.code(),
            "c5_derivatives": ks.c5.code(),
            "overall": ks.overall,
        },
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

fn kind_of(raw: &str) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(raw)).replace(['-', ' '], "_")
}

/// Inbound kinds the journal books as income (and as an acquisition beside
/// it), which are no purchases.
const EARNED: &[&str] = &[
    "airdrop",
    "hard_fork",
    "hardfork",
    "income",
    "interest",
    "lending_interest",
    "mining",
    "mining_reward",
    "routing_income",
    "staking",
];

/// The settings key of a book's KS 36 inputs.
pub fn inputs_key(profile_id: &str) -> String {
    format!("tax_country.ch.ks36:{profile_id}")
}

/// The book's KS 36 inputs (`docs/reference/tax-country-ch.md#using-it`):
/// a JSON object under [`inputs_key`] in `settings`, every field optional.
/// A value Kassiber cannot read is refused rather than ignored.
pub fn inputs(conn: &Connection, profile_id: &str) -> Result<Inputs> {
    let rows = rows(
        conn,
        "SELECT value FROM settings WHERE key=?",
        [inputs_key(profile_id)],
    )?;
    let Some(row) = rows.first() else {
        return Ok(Inputs::default());
    };
    let invalid = |what: &str| {
        AppError::new("validation", format!("Invalid Swiss KS 36 input: {what}"))
            .with_hint("Store a JSON object with net_income_chf, debt_financed_without_cover, derivatives_only_hedging, professional_trader and canton.")
    };
    let parsed: serde_json::Value =
        serde_json::from_str(&text(row, "value")).map_err(|_| invalid("not JSON"))?;
    let object = parsed.as_object().ok_or_else(|| invalid("not an object"))?;
    let flag = |key: &str| -> Result<Option<bool>> {
        match object.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(invalid(key)),
        }
    };
    let mut net_income = BTreeMap::new();
    match object.get("net_income_chf") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::Object(by_year)) => {
            for (year, amount) in by_year {
                let year: i32 = year.parse().map_err(|_| invalid("net_income_chf year"))?;
                let amount: Decimal = amount
                    .as_str()
                    .and_then(|a| a.parse().ok())
                    .ok_or_else(|| invalid("net_income_chf amount"))?;
                net_income.insert(year, amount);
            }
        }
        Some(_) => return Err(invalid("net_income_chf")),
    }
    Ok(Inputs {
        net_income,
        debt_financed_without_cover: flag("debt_financed_without_cover")?,
        derivatives_only_hedging: flag("derivatives_only_hedging")?,
        professional_trader: flag("professional_trader")?.unwrap_or(false),
        canton: match object.get("canton") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(canton)) => Some(canton.clone()),
            Some(_) => return Err(invalid("canton")),
        },
    })
}

/// The year-end values of the rate cache, per year and asset: for pair
/// `<ASSET>-CHF`, the rates timed on 31 December in Zurich; an
/// `estv_kursliste` rate wins, else the day's last rate is the market close.
pub fn valuations(conn: &Connection, zone: &str) -> Result<BTreeMap<(i32, String), Valuation>> {
    let cached = rows(
        conn,
        "SELECT pair,timestamp,rate,rate_exact,source FROM rates_cache \
         WHERE pair LIKE '%-CHF' ORDER BY pair,timestamp,source",
        [],
    )?;
    let mut out: BTreeMap<(i32, String), (bool, String, Valuation)> = BTreeMap::new();
    for row in &cached {
        let Ok((year, date)) = crate::calendar::local(&text(row, "timestamp"), zone) else {
            continue;
        };
        if !date.ends_with("-12-31") {
            continue;
        }
        let pair = text(row, "pair");
        let Some(asset) = pair.strip_suffix("-CHF") else {
            continue;
        };
        let source = text(row, "source");
        let estv = source == ESTV;
        let unit = exact(row, "rate")?;
        let key = (year as i32, asset.to_owned());
        let at = text(row, "timestamp");
        let better = match out.get(&key) {
            None => true,
            Some((was_estv, was_at, _)) => (estv, &at) >= (*was_estv, was_at),
        };
        if better {
            out.insert(
                key,
                (
                    estv,
                    at,
                    Valuation {
                        unit,
                        source: if estv { ESTV.into() } else { MARKET.into() },
                    },
                ),
            );
        }
    }
    Ok(out.into_iter().map(|(key, (_, _, v))| (key, v)).collect())
}

/// The book's disposals, income, outside receipts, purchases and holdings
/// changes from its journal, in journal order; `wallet` limits them to one
/// wallet.
pub fn load(conn: &Connection, profile_id: &str, zone: &str, wallet: Option<&str>) -> Result<Book> {
    let entries = rows(
        conn,
        "SELECT je.transaction_id,je.occurred_at,je.entry_type,je.asset,je.quantity,\
         je.proceeds,je.proceeds_exact,je.cost_basis,je.cost_basis_exact,je.gain_loss,je.gain_loss_exact,\
         je.calculation_json,je.calculation_compact,COALESCE(w.label,'') AS wallet,\
         COALESCE(NULLIF(t.kind_override,''),t.kind,'') AS kind,t.fiat_value,t.fiat_value_exact,t.amount AS transaction_amount \
         FROM journal_entries je LEFT JOIN wallets w ON w.id=je.wallet_id \
         LEFT JOIN transactions t ON t.id=je.transaction_id \
         WHERE je.profile_id=?1 AND (?2 IS NULL OR je.wallet_id=?2) \
         ORDER BY je.occurred_at,je.created_at,je.id",
        kassiber_store::rusqlite::params![profile_id, wallet],
    )?;
    let mut book = Book::default();
    for entry in &entries {
        let entry_type = text(entry, "entry_type");
        let date = crate::calendar::local(&text(entry, "occurred_at"), zone)?.1;
        let transaction_id = text(entry, "transaction_id");
        let wallet = text(entry, "wallet");
        let asset = text(entry, "asset");
        let kind = kind_of(&text(entry, "kind"));
        let signed = btc(int(entry, "quantity"))?;
        // An income entry's units are its acquisition's; a move's fee is in
        // its outbound leg.
        if !["income", "transfer_fee"].contains(&entry_type.as_str()) {
            book.movements.push(Movement {
                date: date.clone(),
                wallet: wallet.clone(),
                asset: asset.clone(),
                quantity: signed.clone(),
            });
        }
        match entry_type.as_str() {
            "income" => book.income.push(Income {
                transaction_id,
                wallet,
                asset,
                date,
                category: category(&kind),
                kind,
                quantity: signed.copy_abs(),
                value: exact(entry, "proceeds")?,
            }),
            "acquisition" => {
                // An acquisition entry carries no value: its transaction's
                // fiat value is the purchase price or the receipt's value.
                let mut value = exact(entry, "fiat_value")?;
                // A slice of the transaction carries its share.
                let whole = int(entry, "transaction_amount").unsigned_abs();
                let part = int(entry, "quantity").unsigned_abs();
                if whole != 0 && part != whole {
                    value = value
                        .mul(&Decimal::from(part), &DECIMAL)
                        .and_then(|v| v.div(&Decimal::from(whole), &DECIMAL))
                        .map_err(arithmetic)?;
                }
                if kind == "wages" || kind.contains("gift") {
                    book.outside.push(Outside {
                        transaction_id,
                        date,
                        kind: if kind == "wages" { "wages" } else { "gift_in" },
                        value,
                    });
                } else if !EARNED.contains(&kind.as_str()) {
                    book.purchases.push((date, value));
                }
            }
            "disposal" => {
                let mut fragments = Vec::new();
                let calculation = crate::gain_explanation::calculation(entry);
                if let Some(Value::List(stored)) =
                    calculation.as_ref().map(|c| value(c, "fragments"))
                {
                    for fragment in stored {
                        let lot = value(fragment, "lot");
                        let at = value(lot, "occurred_at");
                        let acquired = if truthy(at) {
                            crate::calendar::local(&py_str(at), zone)?.1
                        } else {
                            String::new()
                        };
                        let msat = match value(fragment, "quantity_msat") {
                            Value::Int(n) => i64::try_from(n.clone()).unwrap_or(0),
                            _ => 0,
                        };
                        fragments.push(Fragment {
                            lot: text(lot, "transaction_id"),
                            held_6_months: held_6_months(&acquired, &date),
                            acquired,
                            quantity: btc(msat)?.copy_abs(),
                            cost: decimal(value(fragment, "cost_basis_exact"))?,
                            proceeds: decimal(value(fragment, "proceeds_exact"))?,
                            gain: decimal(value(fragment, "gain_loss_exact"))?,
                        });
                    }
                }
                if fragments.is_empty() {
                    fragments.push(Fragment {
                        lot: String::new(),
                        acquired: String::new(),
                        quantity: signed.copy_abs(),
                        cost: exact(entry, "cost_basis")?,
                        proceeds: exact(entry, "proceeds")?,
                        gain: exact(entry, "gain_loss")?,
                        held_6_months: false,
                    });
                }
                match book.disposals.iter_mut().find(|d| {
                    d.transaction_id == transaction_id && d.wallet == wallet && d.asset == asset
                }) {
                    Some(disposal) => disposal.fragments.extend(fragments),
                    None => book.disposals.push(Disposal {
                        transaction_id,
                        wallet,
                        asset,
                        kind,
                        date,
                        fragments,
                    }),
                }
            }
            _ => {}
        }
    }
    for disposal in &mut book.disposals {
        disposal
            .fragments
            .sort_by(|a, b| (&a.acquired, &a.lot).cmp(&(&b.acquired, &b.lot)));
    }
    Ok(book)
}

fn dec(d: &Decimal) -> Value {
    s(d.to_string())
}

fn optional(d: &Option<Decimal>) -> Value {
    d.as_ref().map_or(Value::Null, dec)
}

/// The layer's part of a report model, exact and unformatted. A book marked
/// as a professional trader's is refused (CH-PROFESSIONAL).
pub fn model(conn: &Connection, profile: &Value, wallet: Option<&str>) -> Result<Value> {
    let profile_id = text(profile, "id");
    let country = crate::country::entry(&kassiber_pytext::lower(kassiber_pytext::strip(&text(
        profile,
        "tax_country",
    ))))
    .ok_or_else(|| AppError::new("internal_error", "Unsupported tax country"))?;
    let inputs = inputs(conn, &profile_id)?;
    if inputs.professional_trader {
        return Err(AppError::new(
            "professional_trading_unsupported",
            "This book is marked as a professional trader's: business-asset accounting (Art. 18 DBG, KS 36) is not modelled",
        )
        .with_hint("Kassiber reports a private investor's income, wealth and KS 36 indicators only."));
    }
    let zone = country.timezone;
    let book = load(conn, &profile_id, zone, wallet)?;
    let cache = valuations(conn, zone)?;
    let value = |year: i32, asset: &str| cache.get(&(year, asset.to_owned())).cloned();
    let years = years(&book, &[], &value, &inputs)?;
    let year_value = |y: &Year| -> Value {
        let ks = &y.ks36;
        dict([
            ("year", Value::Int(y.year.into())),
            ("realized_gain", dec(&y.realized_gain)),
            ("income_total", dec(&y.income_total)),
            (
                "income_by_category",
                dict(
                    y.income_by_category
                        .iter()
                        .map(|(k, v)| (*k, dec(v)))
                        .collect::<Vec<_>>(),
                ),
            ),
            ("wealth_total", optional(&y.wealth.total)),
            ("wealth_complete", Value::Bool(y.wealth.total.is_some())),
            (
                "wealth",
                Value::List(
                    y.wealth
                        .rows
                        .iter()
                        .map(|r| {
                            dict([
                                ("wallet", s(r.wallet.as_str())),
                                ("asset", s(r.asset.as_str())),
                                ("quantity", dec(&r.quantity)),
                                (
                                    "unit_value",
                                    r.valuation.as_ref().map_or(Value::Null, |v| dec(&v.unit)),
                                ),
                                ("value", optional(&r.value)),
                                (
                                    "source",
                                    r.valuation
                                        .as_ref()
                                        .map_or(Value::Null, |v| s(v.source.as_str())),
                                ),
                                (
                                    "flags",
                                    Value::List(r.flags.iter().map(|f| s(*f)).collect()),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "ks36",
                dict([
                    ("c1", s(ks.c1.code())),
                    ("c2", s(ks.c2.code())),
                    ("volume", dec(&ks.volume)),
                    ("limit", optional(&ks.limit)),
                    ("c3", s(ks.c3.code())),
                    ("gains", dec(&ks.gains)),
                    ("half_income", optional(&ks.half_income)),
                    ("c4", s(ks.c4.code())),
                    ("c5", s(ks.c5.code())),
                    ("overall", s(ks.overall)),
                ]),
            ),
        ])
    };
    Ok(dict([
        ("country", s(country.code)),
        ("wallet_scoped", Value::Bool(wallet.is_some())),
        ("canton", inputs.canton.as_deref().map_or(Value::Null, s)),
        (
            "inputs_given",
            Value::Bool(
                !inputs.net_income.is_empty()
                    || inputs.debt_financed_without_cover.is_some()
                    || inputs.derivatives_only_hedging.is_some(),
            ),
        ),
        (
            "disposals",
            Value::List(
                book.disposals
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
                                                ("gain", dec(&f.gain)),
                                                ("held_6_months", Value::Bool(f.held_6_months)),
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
                book.income
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
                            ("category", s(i.category)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "outside",
            Value::List(
                book.outside
                    .iter()
                    .map(|o| {
                        dict([
                            ("transaction_id", s(o.transaction_id.as_str())),
                            ("date", s(o.date.as_str())),
                            ("kind", s(o.kind)),
                            ("value", dec(&o.value)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("years", Value::List(years.iter().map(year_value).collect())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn six_months_clamp_to_the_month_end() {
        // ch-10, ch-11.
        assert!(!held_6_months("2024-01-31", "2024-07-30"));
        assert!(held_6_months("2024-01-31", "2024-07-31"));
        assert!(held_6_months("2024-08-31", "2025-02-28"));
        assert!(!held_6_months("2024-08-31", "2025-02-27"));
        assert!(!held_6_months("", "2025-02-27"));
        assert_eq!(category("lending_interest"), MOVABLE);
        assert_eq!(category("airdrop"), MOVABLE);
        assert_eq!(category("routing_income"), GENERAL);
        assert_eq!(category("mining"), MINING);
    }

    fn movement(date: &str, wallet: &str, asset: &str, quantity: &str) -> Movement {
        Movement {
            date: date.into(),
            wallet: wallet.into(),
            asset: asset.into(),
            quantity: d(quantity),
        }
    }

    #[test]
    fn wealth_is_unknown_without_a_value_and_lbtc_borrows_bitcoins() {
        let movements = [
            movement("2024-02-01", "A", "BTC", "0.5"),
            movement("2024-03-01", "A", "BTC", "-0.2"),
            movement("2024-03-01", "A", "LBTC", "0.2"),
            movement("2025-01-01", "A", "TOKZ", "500"),
        ];
        let estv = |year: i32, asset: &str| {
            (year == 2024 && asset == "BTC").then(|| Valuation {
                unit: d("84000"),
                source: ESTV.into(),
            })
        };
        let w = wealth(&movements, 2024, &estv).unwrap();
        assert_eq!(w.total, Some(d("42000.0")));
        assert_eq!(w.rows[1].flags, ["lbtc_valued_as_btc"]);
        let w = wealth(&movements, 2025, &estv).unwrap();
        assert_eq!(w.total, None);
        assert!(w
            .rows
            .iter()
            .all(|r| r.flags.contains(&"missing_valuation")));
        // A market close is flagged as not the ESTV tax value.
        let market = |_: i32, _: &str| {
            Some(Valuation {
                unit: d("85210.33"),
                source: MARKET.into(),
            })
        };
        let w = wealth(&movements[..1], 2024, &market).unwrap();
        assert_eq!(cents(w.total.as_ref().unwrap()).unwrap(), "42605.16");
        assert_eq!(w.rows[0].flags, ["not_estv_tax_value"]);
    }

    #[test]
    fn year_end_values_and_inputs_come_from_the_book() {
        let dir = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&dir.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(
            "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES
             ('BTC-CHF','2024-12-31T12:00:00+01:00',84000,'84000','estv_kursliste','x'),
             ('BTC-CHF','2024-12-31T22:00:00Z',85000,'85000','coingecko','x'),
             ('BTC-CHF','2024-12-31T23:30:00Z',99000,'99000','coingecko','x'),
             ('BTC-CHF','2023-12-31T10:00:00Z',40000,'40000','kraken','x'),
             ('BTC-CHF','2023-12-31T18:00:00Z',41000,'41000','kraken','x'),
             ('BTC-CHF','2023-12-30T18:00:00Z',39000,'39000','estv_kursliste','x'),
             ('BTC-EUR','2024-12-31T12:00:00Z',1,'1','estv_kursliste','x');",
        )
        .unwrap();
        let values = valuations(conn, "Europe/Zurich").unwrap();
        // The ESTV value of 31 December wins over the day's later rates;
        // 23:30 UTC is already 1 January in Zurich.
        let v = &values[&(2024, "BTC".to_owned())];
        assert_eq!(
            (v.unit.to_string().as_str(), v.source.as_str()),
            ("84000", ESTV)
        );
        // Without one, the day's last rate is the market close; a rate of
        // another day is no year-end value.
        let v = &values[&(2023, "BTC".to_owned())];
        assert_eq!(
            (v.unit.to_string().as_str(), v.source.as_str()),
            ("41000", MARKET)
        );
        assert_eq!(values.len(), 2);

        assert!(!inputs(conn, "p").unwrap().professional_trader);
        let put = |json: &str| {
            conn.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
                kassiber_store::rusqlite::params![inputs_key("p"), json],
            )
            .unwrap();
        };
        put(
            r#"{"net_income_chf": {"2024": "120000"}, "derivatives_only_hedging": true,
                "professional_trader": true, "canton": "Bern"}"#,
        );
        let read = inputs(conn, "p").unwrap();
        assert_eq!(read.net_income[&2024], d("120000"));
        assert_eq!(
            (
                read.derivatives_only_hedging,
                read.debt_financed_without_cover
            ),
            (Some(true), None)
        );
        assert!(read.professional_trader);
        assert_eq!(read.canton.as_deref(), Some("Bern"));
        // An input Kassiber cannot read is refused, not ignored.
        put(r#"{"professional_trader": "yes"}"#);
        assert_eq!(inputs(conn, "p").unwrap_err().code, "validation");
        put("[]");
        assert_eq!(inputs(conn, "p").unwrap_err().code, "validation");
        // A professional trader's book is refused by the layer.
        put(r#"{"professional_trader": true}"#);
        let profile = dict([("id", s("p")), ("tax_country", s("ch"))]);
        assert_eq!(
            model(conn, &profile, None).unwrap_err().code,
            "professional_trading_unsupported"
        );
    }

    #[test]
    fn ks36_needs_its_inputs_and_fails_on_any_unmet_criterion() {
        let book = Book {
            disposals: vec![Disposal {
                transaction_id: "s1".into(),
                wallet: "A".into(),
                asset: "BTC".into(),
                kind: "sell".into(),
                date: "2024-03-01".into(),
                fragments: vec![Fragment {
                    lot: "b1".into(),
                    acquired: "2023-01-10".into(),
                    quantity: d("1"),
                    cost: d("20000"),
                    proceeds: d("50000"),
                    gain: d("30000"),
                    held_6_months: true,
                }],
            }],
            purchases: vec![("2023-01-10".into(), d("20000"))],
            movements: vec![
                movement("2023-01-10", "A", "BTC", "1"),
                movement("2024-03-01", "A", "BTC", "-1"),
            ],
            ..Book::default()
        };
        let value = |year: i32, _: &str| {
            (year == 2023).then(|| Valuation {
                unit: d("38000"),
                source: ESTV.into(),
            })
        };
        let years = years(&book, &[], &value, &Inputs::default()).unwrap();
        let json = year_json(&years[1]).unwrap();
        assert_eq!(json["ks36"]["c2_volume_5x"]["limit_chf"], "190000.00");
        assert_eq!(
            json["ks36"]["c3_gains_under_half_income"]["result"],
            "not_assessable"
        );
        assert_eq!(json["ks36"]["overall"], "inconclusive");
        // ch-14: gains equal to half the net income are not under half.
        let inputs = Inputs {
            net_income: [(2023, d("60000")), (2024, d("60000"))].into(),
            debt_financed_without_cover: Some(false),
            derivatives_only_hedging: Some(true),
            ..Inputs::default()
        };
        let years = super::years(&book, &[], &value, &inputs).unwrap();
        let json = year_json(&years[1]).unwrap();
        assert_eq!(
            json["ks36"]["c3_gains_under_half_income"]["result"],
            "not_met"
        );
        assert_eq!(json["ks36"]["overall"], "not_met");
        assert_eq!(year_json(&years[0]).unwrap()["ks36"]["c4_financing"], "met");
        assert_eq!(json["taxable_capital_gains_chf"], "0.00");
    }
}
