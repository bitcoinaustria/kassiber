//! The United Kingdom's country layer data: the engine's share matching
//! grouped into one computation per token and UK day, its matches (same
//! day, 30 days, section 104), the section 104 pools, and per tax year the
//! SA108 boxes, the annual exempt amount, the reporting tests and 2024-25's
//! split at 30 October (`docs/reference/tax-country-gb.md`). The engine
//! decides every match and gain (`uk_share_matching`); this module groups,
//! totals and places them, and maps the transactions the engine held to the
//! United Kingdom's refusal codes.

use super::*;
use kassiber_tax_countries::calendar::{self, Date};
use kassiber_tax_countries::tables;
use kassiber_tax_countries::years;
use std::collections::{BTreeMap, BTreeSet};

/// The tax year's first day.
pub const TAX_YEAR_START: (u8, u8) = (4, 6);
/// The first tax year with the SA108 cryptoasset boxes (13.1 to 13.8).
pub const FIRST_CRYPTO_BOXES_YEAR: i32 = 2024;

/// The part of a disposal one acquisition date (or the pool) covered, as
/// the engine's fragment says it.
#[derive(Clone, Debug)]
pub struct Piece {
    /// The London date of the fragment's lot.
    pub acquired: Date,
    pub quantity: Decimal,
    pub cost: Decimal,
}

/// One disposal entry the engine computed.
#[derive(Clone, Debug)]
pub struct Disposal {
    /// The computation's name for it: the transaction, or `<id>:fee` for a
    /// move's network fee.
    pub event: String,
    pub wallet: String,
    pub asset: String,
    /// The London date.
    pub date: Date,
    pub quantity: Decimal,
    pub proceeds: Decimal,
    pub cost: Decimal,
    /// Costs the engine deducted from the gain: a sale's fee tokens at
    /// their value (GB-FEE-TOKENS).
    pub expenses: Decimal,
    /// Incidental costs the book records apart from the value; a Kassiber
    /// book nets a fiat commission into its value, so they are zero there.
    pub incidental: Decimal,
    pub pieces: Vec<Piece>,
}

/// One acquisition: a lot of the pool's token.
#[derive(Clone, Debug)]
pub struct Acquisition {
    pub asset: String,
    pub date: Date,
    pub quantity: Decimal,
    pub cost: Decimal,
}

/// One receipt of miscellaneous income.
#[derive(Clone, Debug)]
pub struct Income {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    pub kind: String,
    pub date: Date,
    pub quantity: Decimal,
    pub value: Decimal,
}

/// A transaction the United Kingdom's rules refuse to guess about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub transaction_id: String,
    pub code: String,
}

/// Which rule matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    SameDay,
    ThirtyDay,
    S104,
}

impl Rule {
    pub fn code(self) -> &'static str {
        match self {
            Rule::SameDay => "same_day",
            Rule::ThirtyDay => "thirty_day",
            Rule::S104 => "s104",
        }
    }
}

/// One match of a computation.
#[derive(Clone, Debug)]
pub struct Match {
    pub rule: Rule,
    /// The acquisition day; none for the pool.
    pub acquired: Option<Date>,
    pub quantity: Decimal,
    pub cost: Decimal,
    /// The pool's quantity and cost before a section 104 match.
    pub pool_before: Option<(Decimal, Decimal)>,
}

/// One token's disposals of one UK day: one disposal for the rules.
#[derive(Clone, Debug)]
pub struct Computation {
    pub asset: String,
    pub date: Date,
    pub tax_year: i32,
    pub events: Vec<String>,
    pub wallets: Vec<String>,
    pub quantity: Decimal,
    pub proceeds: Decimal,
    pub allowable_costs: Decimal,
    pub incidental: Decimal,
    pub matches: Vec<Match>,
    pub gain: Decimal,
}

/// A token's section 104 pool at a moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    pub asset: String,
    pub quantity: Decimal,
    pub cost: Decimal,
}

/// One tax year's SA108 figures.
#[derive(Clone, Debug)]
pub struct Year {
    pub year: i32,
    pub disposals: usize,
    pub proceeds: Decimal,
    pub allowable_costs: Decimal,
    pub gains: Decimal,
    pub losses: Decimal,
    pub net_gains: Decimal,
    pub annual_exempt_amount: Option<Decimal>,
    pub proceeds_threshold: Option<Decimal>,
    pub misc_income: Decimal,
    /// The date the main rates changed within the year, with the gains and
    /// losses before and from it.
    pub split: Option<(Date, [Decimal; 4])>,
    /// The last disposal day: a purchase within 30 days after it could
    /// still change the figures.
    pub last_disposal: Option<Date>,
    /// Each token's pool at the end of the year.
    pub pools: Vec<Pool>,
}

fn add(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.add(b, &DECIMAL).map_err(arithmetic)
}
fn sub(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.sub(b, &DECIMAL).map_err(arithmetic)
}

/// The London date of an instant; Kassiber has no London calendar before
/// 1996, so an older instant is refused rather than given a guessed offset.
pub fn london_date(us: i64) -> Result<Date> {
    calendar::zone(kassiber_tax_countries::LONDON)
        .expect("the registry has London's calendar")
        .local_date(us)
        .map_err(|error| AppError::new("validation", error.to_string()))
}

/// The tax year of a London date and its label (`2024-25`).
pub fn tax_year(date: Date) -> (i32, String) {
    let year = years::tax_year(date, TAX_YEAR_START);
    (year, years::tax_year_label(year, TAX_YEAR_START))
}

/// Each token's pool at the end of every day with a movement.
pub type PoolHistory = BTreeMap<String, Vec<(Date, Pool)>>;

/// The computations, by token then day, and each token's pool at the end
/// of every day with a movement. A computation's matches come from its
/// fragments' lot dates: the same day, a later day (30 days), or an
/// earlier day (the pool, which only ever holds earlier acquisitions).
pub fn computations(
    disposals: &[Disposal],
    acquisitions: &[Acquisition],
) -> Result<(Vec<Computation>, PoolHistory)> {
    let mut grouped: BTreeMap<(String, Date), Vec<&Disposal>> = BTreeMap::new();
    for disposal in disposals {
        grouped
            .entry((disposal.asset.clone(), disposal.date))
            .or_default()
            .push(disposal);
    }
    let mut out = Vec::new();
    // Per token and acquisition day, what same-day and 30-day matches took.
    let mut matched: BTreeMap<(String, Date), (Decimal, Decimal)> = BTreeMap::new();
    for ((asset, date), group) in &grouped {
        let mut computation = Computation {
            asset: asset.clone(),
            date: *date,
            tax_year: tax_year(*date).0,
            events: Vec::new(),
            wallets: Vec::new(),
            quantity: Decimal::zero(),
            proceeds: Decimal::zero(),
            allowable_costs: Decimal::zero(),
            incidental: Decimal::zero(),
            matches: Vec::new(),
            gain: Decimal::zero(),
        };
        let mut same = (Decimal::zero(), Decimal::zero());
        let mut thirty: BTreeMap<Date, (Decimal, Decimal)> = BTreeMap::new();
        let mut pool = (Decimal::zero(), Decimal::zero());
        let mut cost = Decimal::zero();
        for disposal in group {
            if !computation.events.contains(&disposal.event) {
                computation.events.push(disposal.event.clone());
            }
            if !computation.wallets.contains(&disposal.wallet) {
                computation.wallets.push(disposal.wallet.clone());
            }
            computation.quantity = add(&computation.quantity, &disposal.quantity)?;
            computation.proceeds = add(
                &computation.proceeds,
                &add(&disposal.proceeds, &disposal.incidental)?,
            )?;
            computation.incidental = add(
                &computation.incidental,
                &add(&disposal.expenses, &disposal.incidental)?,
            )?;
            cost = add(&cost, &disposal.cost)?;
            for piece in &disposal.pieces {
                let slot = match piece.acquired.cmp(date) {
                    std::cmp::Ordering::Equal => &mut same,
                    std::cmp::Ordering::Greater => thirty
                        .entry(piece.acquired)
                        .or_insert_with(|| (Decimal::zero(), Decimal::zero())),
                    std::cmp::Ordering::Less => &mut pool,
                };
                slot.0 = add(&slot.0, &piece.quantity)?;
                slot.1 = add(&slot.1, &piece.cost)?;
                if piece.acquired >= *date {
                    let taken = matched
                        .entry((asset.clone(), piece.acquired))
                        .or_insert_with(|| (Decimal::zero(), Decimal::zero()));
                    taken.0 = add(&taken.0, &piece.quantity)?;
                    taken.1 = add(&taken.1, &piece.cost)?;
                }
            }
        }
        if same.0.is_positive() {
            computation.matches.push(Match {
                rule: Rule::SameDay,
                acquired: Some(*date),
                quantity: same.0,
                cost: same.1,
                pool_before: None,
            });
        }
        for (acquired, (quantity, cost)) in thirty {
            computation.matches.push(Match {
                rule: Rule::ThirtyDay,
                acquired: Some(acquired),
                quantity,
                cost,
                pool_before: None,
            });
        }
        if pool.0.is_positive() {
            computation.matches.push(Match {
                rule: Rule::S104,
                acquired: None,
                quantity: pool.0,
                cost: pool.1,
                pool_before: None,
            });
        }
        computation.allowable_costs = add(&cost, &computation.incidental)?;
        computation.gain = sub(&computation.proceeds, &computation.allowable_costs)?;
        out.push(computation);
    }

    // The pools, day by day: a day's unmatched acquisitions join before its
    // section 104 match takes from it.
    let mut joining: BTreeMap<(String, Date), (Decimal, Decimal)> = BTreeMap::new();
    for acquisition in acquisitions {
        let slot = joining
            .entry((acquisition.asset.clone(), acquisition.date))
            .or_insert_with(|| (Decimal::zero(), Decimal::zero()));
        slot.0 = add(&slot.0, &acquisition.quantity)?;
        slot.1 = add(&slot.1, &acquisition.cost)?;
    }
    let assets: BTreeSet<String> = joining
        .keys()
        .map(|(asset, _)| asset.clone())
        .chain(out.iter().map(|c| c.asset.clone()))
        .collect();
    let mut history: PoolHistory = BTreeMap::new();
    for asset in assets {
        let mut days: BTreeSet<Date> = joining
            .keys()
            .filter(|(a, _)| *a == asset)
            .map(|(_, d)| *d)
            .collect();
        days.extend(out.iter().filter(|c| c.asset == asset).map(|c| c.date));
        let mut quantity = Decimal::zero();
        let mut cost = Decimal::zero();
        let mut states = Vec::new();
        for day in days {
            if let Some((q, c)) = joining.get(&(asset.clone(), day)) {
                let (mq, mc) = matched
                    .get(&(asset.clone(), day))
                    .cloned()
                    .unwrap_or_else(|| (Decimal::zero(), Decimal::zero()));
                quantity = add(&quantity, &sub(q, &mq)?)?;
                cost = add(&cost, &sub(c, &mc)?)?;
            }
            if let Some(computation) = out.iter_mut().find(|c| c.asset == asset && c.date == day) {
                if let Some(pool_match) = computation
                    .matches
                    .iter_mut()
                    .find(|m| m.rule == Rule::S104)
                {
                    pool_match.pool_before = Some((quantity.clone(), cost.clone()));
                    quantity = sub(&quantity, &pool_match.quantity)?;
                    cost = sub(&cost, &pool_match.cost)?;
                }
            }
            states.push((
                day,
                Pool {
                    asset: asset.clone(),
                    quantity: quantity.clone(),
                    cost: cost.clone(),
                },
            ));
        }
        history.insert(asset, states);
    }
    Ok((out, history))
}

/// Each token's pool at the end of `last` (tokens acquired by then).
fn pools_at(history: &PoolHistory, last: Date) -> Vec<Pool> {
    history
        .values()
        .filter_map(|states| {
            states
                .iter()
                .rev()
                .find(|(day, _)| *day <= last)
                .map(|(_, pool)| pool.clone())
        })
        .collect()
}

/// Each token's pool after every movement.
pub fn final_pools(history: &PoolHistory) -> Vec<Pool> {
    history
        .values()
        .filter_map(|states| states.last().map(|(_, pool)| pool.clone()))
        .collect()
}

fn lookup_value(table: &years::Schedule, date: Date) -> Result<Option<Decimal>> {
    Ok(crate::tax_years::table_value(table, date)?.ok())
}

/// The tax years with a computation or income, in order.
pub fn years(
    computations: &[Computation],
    income: &[Income],
    history: &PoolHistory,
) -> Result<Vec<Year>> {
    let mut labels: BTreeSet<i32> = computations.iter().map(|c| c.tax_year).collect();
    labels.extend(income.iter().map(|i| tax_year(i.date).0));
    let mut out = Vec::new();
    for year in labels {
        let (first, last) = years::tax_year_bounds(year, TAX_YEAR_START);
        let in_year: Vec<&Computation> =
            computations.iter().filter(|c| c.tax_year == year).collect();
        let mut item = Year {
            year,
            disposals: in_year.len(),
            proceeds: Decimal::zero(),
            allowable_costs: Decimal::zero(),
            gains: Decimal::zero(),
            losses: Decimal::zero(),
            net_gains: Decimal::zero(),
            annual_exempt_amount: lookup_value(&tables::GB_ANNUAL_EXEMPT_AMOUNT, first)?,
            proceeds_threshold: lookup_value(&tables::GB_PROCEEDS_THRESHOLD, first)?,
            misc_income: Decimal::zero(),
            split: None,
            last_disposal: in_year.iter().map(|c| c.date).max(),
            pools: pools_at(history, last),
        };
        let mut split = tables::GB_CGT_MAIN_RATES
            .changes_within(first, last)
            .first()
            .map(|day| {
                (
                    *day,
                    [
                        Decimal::zero(),
                        Decimal::zero(),
                        Decimal::zero(),
                        Decimal::zero(),
                    ],
                )
            });
        for c in &in_year {
            item.proceeds = add(&item.proceeds, &c.proceeds)?;
            item.allowable_costs = add(&item.allowable_costs, &c.allowable_costs)?;
            let (slot, index) = if c.gain.is_negative() {
                (&mut item.losses, 1)
            } else {
                (&mut item.gains, 0)
            };
            *slot = add(slot, &c.gain.copy_abs())?;
            if let Some((day, figures)) = &mut split {
                let at = index + if c.date >= *day { 2 } else { 0 };
                figures[at] = add(&figures[at], &c.gain.copy_abs())?;
            }
        }
        item.split = split;
        item.net_gains = sub(&item.gains, &item.losses)?;
        for i in income.iter().filter(|i| tax_year(i.date).0 == year) {
            item.misc_income = add(&item.misc_income, &i.value)?;
        }
        out.push(item);
    }
    Ok(out)
}

impl Year {
    pub fn label(&self) -> String {
        years::tax_year_label(self.year, TAX_YEAR_START)
    }
    /// The SA108 section the year's crypto goes in.
    pub fn crypto_boxes(&self) -> bool {
        self.year >= FIRST_CRYPTO_BOXES_YEAR
    }
    /// Net gains above the annual exempt amount, when it is on file.
    pub fn gains_above_aea(&self) -> Result<Option<Decimal>> {
        self.annual_exempt_amount
            .as_ref()
            .map(|aea| {
                let above = sub(&self.net_gains, aea)?;
                Ok(if above.is_negative() {
                    Decimal::zero()
                } else {
                    above
                })
            })
            .transpose()
    }
    /// A net loss for the year, which can be claimed.
    pub fn loss_available(&self) -> Decimal {
        if self.net_gains.is_negative() {
            self.net_gains.copy_abs()
        } else {
            Decimal::zero()
        }
    }
    /// The three reporting tests (GB-REPORT-THRESHOLD): proceeds over the
    /// threshold, gains before losses over the AEA, a loss to claim; `None`
    /// where the year's table is not on file.
    pub fn report_required(&self) -> (Option<bool>, Option<bool>, bool) {
        (
            self.proceeds_threshold
                .as_ref()
                .map(|threshold| &self.proceeds > threshold),
            self.annual_exempt_amount
                .as_ref()
                .map(|aea| &self.gains > aea),
            self.losses.is_positive(),
        )
    }
}

/// A fiat amount half-even at cents, as the vectors and boxes' notes write
/// it; a negative amount that rounds to nothing is `0.00`.
pub fn money(value: &Decimal) -> Result<String> {
    let text = value
        .to_fixed_places(2, Rounding::HalfEven)
        .map_err(arithmetic)?;
    Ok(if text == "-0.00" { "0.00".into() } else { text })
}

/// The engine's 32-digit arithmetic leaves dust far below a penny where a
/// pool's average cost is divided and multiplied again (2E-28 on 50,000);
/// ten places keep every real fraction of a penny and drop the dust.
fn without_dust(value: &Decimal) -> Result<Decimal> {
    let rounded: Decimal = value
        .to_fixed_places(10, Rounding::HalfEven)
        .map_err(arithmetic)?
        .parse()
        .map_err(|_| AppError::new("internal_error", "invalid decimal"))?;
    Ok(if rounded.is_zero() {
        Decimal::zero()
    } else {
        rounded
    })
}

/// A box value: the pence dropped (Q1), after the engine's dust.
pub fn pounds(value: &Decimal) -> Result<String> {
    without_dust(value)?
        .to_fixed_places(0, Rounding::Down)
        .map_err(arithmetic)
}

/// The expenses an entry deducted: its proceeds less its cost and gain.
pub fn expenses_of(proceeds: &Decimal, cost: &Decimal, gain: &Decimal) -> Result<Decimal> {
    without_dust(&sub(&sub(proceeds, cost)?, gain)?)
}

/// The figures in the shared vector schema
/// (`docs/reference/tax-country-gb-vectors.json`).
pub fn vector_json(
    computations: &[Computation],
    income: &[Income],
    years: &[Year],
    pools: &[Pool],
) -> Result<serde_json::Value> {
    use serde_json::json;
    let quantity = crate::ie_tax::quantity_text;
    let mut out_computations = Vec::new();
    for c in computations {
        let mut matches = Vec::new();
        for m in &c.matches {
            let mut item = json!({"rule": m.rule.code(), "acquired": m.acquired.map(|d| d.iso()),
                                  "quantity": quantity(&m.quantity)?, "cost": money(&m.cost)?});
            if let Some((q, cost)) = &m.pool_before {
                item["pool_before"] = json!([quantity(q)?, money(cost)?]);
            }
            matches.push(item);
        }
        out_computations.push(json!({
            "asset": c.asset, "date": c.date.iso(), "tax_year": years::tax_year_label(c.tax_year, TAX_YEAR_START),
            "events": c.events, "quantity": quantity(&c.quantity)?, "proceeds": money(&c.proceeds)?,
            "allowable_costs": money(&c.allowable_costs)?, "incidental": money(&c.incidental)?,
            "matches": matches, "gain": money(&c.gain)?,
        }));
    }
    let mut out_income = Vec::new();
    for i in income {
        out_income.push(json!({"event": i.transaction_id, "date": i.date.iso(), "tax_year": tax_year(i.date).1,
                               "subkind": i.kind, "quantity": quantity(&i.quantity)?, "value": money(&i.value)?}));
    }
    let mut out_years = serde_json::Map::new();
    for y in years {
        let aea = y.annual_exempt_amount.as_ref().map(money).transpose()?;
        let (over_threshold, over_aea, loss) = y.report_required();
        let mut entry = json!({
            "sa108_section": if y.crypto_boxes() { "cryptoassets boxes 13.1-13.5" } else { "other property, assets and gains boxes 14-17, 19" },
            "disposals": y.disposals, "proceeds": money(&y.proceeds)?, "allowable_costs": money(&y.allowable_costs)?,
            "gains": money(&y.gains)?, "losses": money(&y.losses)?, "net_gains": money(&y.net_gains)?,
            "annual_exempt_amount": aea,
            "gains_above_aea": y.gains_above_aea()?.as_ref().map(money).transpose()?,
            "loss_available": money(&y.loss_available())?,
            "boxes": {"count": y.disposals, "proceeds": pounds(&y.proceeds)?, "costs": pounds(&y.allowable_costs)?,
                      "gains": pounds(&y.gains)?, "losses": pounds(&y.losses)?},
            "report_required": {"proceeds_over_threshold": over_threshold, "gains_over_aea": over_aea, "loss_to_claim": loss},
            "misc_income_box17": money(&y.misc_income)?,
        });
        if y.annual_exempt_amount.is_none() {
            entry["status"] = json!("aea_unknown_for_year");
        }
        if let Some((_, [gains_before, losses_before, gains_from, losses_from])) = &y.split {
            entry["rate_split_2024_25"] = json!({
                "gains_before_30_oct": money(gains_before)?, "losses_before_30_oct": money(losses_before)?,
                "gains_from_30_oct": money(gains_from)?, "losses_from_30_oct": money(losses_from)?,
            });
        }
        out_years.insert(y.label(), entry);
    }
    let mut out_pools = serde_json::Map::new();
    for pool in pools {
        out_pools.insert(
            pool.asset.clone(),
            json!({"quantity": quantity(&pool.quantity)?, "cost": money(&pool.cost)?}),
        );
    }
    Ok(
        json!({"computations": out_computations, "income": out_income, "years": out_years, "pools": out_pools}),
    )
}

/// The United Kingdom's code for a transaction the engine held
/// (GB-FAILCLOSED): a gift or donation without its recipient class, a
/// receipt it does not book (an unclassified airdrop, a hard fork,
/// unlabelled income), a disposal larger than the holdings; any other hold
/// keeps the engine's reason.
pub fn quarantine_code(reason: &str, kind: &str, code: Option<&str>) -> String {
    if let Some(code) = code {
        return code.into();
    }
    if reason == "non_sale_disposal_kind" && ["gift", "donat"].iter().any(|t| kind.contains(t)) {
        return "gift_recipient_classification_required".into();
    }
    match reason {
        "insufficient_lots" => "insufficient_holdings".into(),
        other => other.into(),
    }
}

/// What the layer reads from the journal.
pub struct Loaded {
    pub disposals: Vec<Disposal>,
    pub acquisitions: Vec<Acquisition>,
    pub income: Vec<Income>,
    pub refusals: Vec<Refusal>,
}

/// The book's disposals, acquisitions, income and refusals, from its
/// journal and quarantines. Matching is the book's (one pool across every
/// wallet), so a `wallet` id limits only what is listed: the computations
/// stay the book's.
pub fn load(conn: &Connection, profile_id: &str) -> Result<Loaded> {
    let entries = rows(
        conn,
        "SELECT je.transaction_id,je.occurred_at,je.entry_type,je.asset,je.quantity,\
         je.fiat_value,je.fiat_value_exact,je.proceeds,je.proceeds_exact,je.cost_basis,je.cost_basis_exact,\
         je.gain_loss,je.gain_loss_exact,je.calculation_json,je.calculation_compact,\
         COALESCE(w.label,'') AS wallet,COALESCE(NULLIF(t.kind_override,''),t.kind,'') AS kind \
         FROM journal_entries je LEFT JOIN wallets w ON w.id=je.wallet_id \
         LEFT JOIN transactions t ON t.id=je.transaction_id \
         WHERE je.profile_id=?1 \
         AND je.entry_type IN ('acquisition','disposal','income','fee','transfer_fee') \
         ORDER BY je.occurred_at,je.created_at,je.id",
        kassiber_store::rusqlite::params![profile_id],
    )?;
    let mut loaded = Loaded {
        disposals: Vec::new(),
        acquisitions: Vec::new(),
        income: Vec::new(),
        refusals: Vec::new(),
    };
    for entry in &entries {
        let date = london_date(crate::layer_reads::instant(&text(entry, "occurred_at"))?)?;
        let transaction_id = text(entry, "transaction_id");
        let asset = text(entry, "asset");
        let quantity = btc(int(entry, "quantity"))?.copy_abs();
        let exact = |key: &str| crate::layer_reads::exact(entry, key);
        match text(entry, "entry_type").as_str() {
            "acquisition" => loaded.acquisitions.push(Acquisition {
                asset,
                date,
                quantity,
                cost: exact("fiat_value")?,
            }),
            "income" => loaded.income.push(Income {
                transaction_id,
                wallet: text(entry, "wallet"),
                asset,
                kind: kassiber_pytext::lower(kassiber_pytext::strip(&text(entry, "kind"))),
                date,
                quantity,
                value: exact("proceeds")?,
            }),
            entry_type => {
                let mut pieces = Vec::new();
                if let Some(calculation) = crate::gain_explanation::calculation(entry) {
                    if let Value::List(fragments) = value(&calculation, "fragments") {
                        for fragment in fragments {
                            let lot = value(fragment, "lot");
                            if matches!(lot, Value::Null) {
                                continue;
                            }
                            pieces.push(Piece {
                                acquired: london_date(crate::layer_reads::instant(&text(
                                    lot,
                                    "occurred_at",
                                ))?)?,
                                quantity: btc(int(fragment, "quantity_msat"))?.copy_abs(),
                                cost: decimal(value(fragment, "cost_basis_exact"))?,
                            });
                        }
                    }
                }
                let proceeds = exact("proceeds")?;
                let cost = exact("cost_basis")?;
                // The entry's gain already deducts its expenses.
                let expenses = expenses_of(&proceeds, &cost, &exact("gain_loss")?)?;
                loaded.disposals.push(Disposal {
                    event: if entry_type == "transfer_fee" {
                        format!("{transaction_id}:fee")
                    } else {
                        transaction_id
                    },
                    wallet: text(entry, "wallet"),
                    asset,
                    date,
                    quantity,
                    proceeds,
                    cost,
                    expenses,
                    incidental: Decimal::zero(),
                    pieces,
                });
            }
        }
    }
    loaded.refusals = crate::layer_reads::quarantines(conn, profile_id, None)?
        .into_iter()
        .map(|held| Refusal {
            code: quarantine_code(&held.reason, &held.kind, held.code.as_deref()),
            transaction_id: held.transaction_id,
        })
        .collect();
    Ok(loaded)
}

fn dec(d: &Decimal) -> Value {
    s(d.to_string())
}
fn opt(d: &Option<Decimal>) -> Value {
    d.as_ref().map_or(Value::Null, dec)
}

/// The layer's part of a report model, exact and unformatted. A book with
/// a refusal shows the refusals and withholds every figure.
pub fn model(conn: &Connection, profile: &Value, wallet: Option<&str>) -> Result<Value> {
    let loaded = load(conn, &text(profile, "id"))?;
    let refusals = Value::List(
        loaded
            .refusals
            .iter()
            .map(|r| {
                dict([
                    ("transaction_id", s(r.transaction_id.as_str())),
                    ("code", s(r.code.as_str())),
                ])
            })
            .collect(),
    );
    let mut model = dict([
        ("country", s("gb")),
        ("wallet_scoped", Value::Bool(wallet.is_some())),
        ("refusals", refusals),
    ]);
    if !loaded.refusals.is_empty() {
        return Ok(model);
    }
    let (computations, history) = computations(&loaded.disposals, &loaded.acquisitions)?;
    let years = years(&computations, &loaded.income, &history)?;
    let pool_value = |p: &Pool| {
        dict([
            ("asset", s(p.asset.as_str())),
            ("quantity", dec(&p.quantity)),
            ("cost", dec(&p.cost)),
        ])
    };
    let computation_values = computations
        .iter()
        .map(|c| {
            let matches = c
                .matches
                .iter()
                .map(|m| {
                    dict([
                        ("rule", s(m.rule.code())),
                        ("acquired", m.acquired.map_or(Value::Null, |d| s(d.iso()))),
                        ("quantity", dec(&m.quantity)),
                        ("cost", dec(&m.cost)),
                        (
                            "pool_quantity_before",
                            m.pool_before.as_ref().map_or(Value::Null, |p| dec(&p.0)),
                        ),
                        (
                            "pool_cost_before",
                            m.pool_before.as_ref().map_or(Value::Null, |p| dec(&p.1)),
                        ),
                    ])
                })
                .collect();
            dict([
                ("asset", s(c.asset.as_str())),
                ("date", s(c.date.iso())),
                ("tax_year", s(tax_year(c.date).1)),
                (
                    "events",
                    Value::List(c.events.iter().map(|e| s(e.as_str())).collect()),
                ),
                (
                    "wallets",
                    Value::List(c.wallets.iter().map(|w| s(w.as_str())).collect()),
                ),
                ("quantity", dec(&c.quantity)),
                ("proceeds", dec(&c.proceeds)),
                ("allowable_costs", dec(&c.allowable_costs)),
                ("incidental", dec(&c.incidental)),
                ("gain", dec(&c.gain)),
                ("matches", Value::List(matches)),
            ])
        })
        .collect();
    let mut year_values = Vec::new();
    for y in &years {
        let (over_threshold, over_aea, loss) = y.report_required();
        let flag = |b: Option<bool>| b.map_or(Value::Null, Value::Bool);
        year_values.push(dict([
            ("year", Value::Int(y.year.into())),
            ("label", s(y.label())),
            ("crypto_boxes", Value::Bool(y.crypto_boxes())),
            ("disposals", Value::Int(y.disposals.into())),
            ("proceeds", dec(&y.proceeds)),
            ("allowable_costs", dec(&y.allowable_costs)),
            ("gains", dec(&y.gains)),
            ("losses", dec(&y.losses)),
            ("net_gains", dec(&y.net_gains)),
            ("annual_exempt_amount", opt(&y.annual_exempt_amount)),
            ("gains_above_aea", opt(&y.gains_above_aea()?)),
            ("loss_available", dec(&y.loss_available())),
            ("proceeds_threshold", opt(&y.proceeds_threshold)),
            ("proceeds_over_threshold", flag(over_threshold)),
            ("gains_over_aea", flag(over_aea)),
            ("loss_to_claim", Value::Bool(loss)),
            ("misc_income", dec(&y.misc_income)),
            (
                "split_date",
                y.split.as_ref().map_or(Value::Null, |(d, _)| s(d.iso())),
            ),
            (
                "split",
                y.split.as_ref().map_or(Value::Null, |(_, f)| {
                    Value::List(f.iter().map(dec).collect())
                }),
            ),
            (
                "last_disposal",
                y.last_disposal.map_or(Value::Null, |d| s(d.iso())),
            ),
            (
                "pools",
                Value::List(y.pools.iter().map(pool_value).collect()),
            ),
        ]));
    }
    replace(&mut model, "computations", Value::List(computation_values));
    replace(
        &mut model,
        "income",
        Value::List(
            loaded
                .income
                .iter()
                .map(|i| {
                    dict([
                        ("transaction_id", s(i.transaction_id.as_str())),
                        ("wallet", s(i.wallet.as_str())),
                        ("asset", s(i.asset.as_str())),
                        ("date", s(i.date.iso())),
                        ("tax_year", s(tax_year(i.date).1)),
                        ("kind", s(i.kind.as_str())),
                        ("quantity", dec(&i.quantity)),
                        ("value", dec(&i.value)),
                    ])
                })
                .collect(),
        ),
    );
    replace(&mut model, "years", Value::List(year_values));
    replace(
        &mut model,
        "pools",
        Value::List(final_pools(&history).iter().map(pool_value).collect()),
    );
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_take_the_united_kingdoms_codes() {
        let code = |reason, kind, code| quarantine_code(reason, kind, code);
        assert_eq!(
            code("non_sale_disposal_kind", "gift", None),
            "gift_recipient_classification_required"
        );
        assert_eq!(
            code("non_sale_disposal_kind", "donation", None),
            "gift_recipient_classification_required"
        );
        // A refused receipt carries the country's code.
        assert_eq!(
            code(
                "acquisition_valuation_unsupported",
                "airdrop",
                Some("airdrop_classification_required")
            ),
            "airdrop_classification_required"
        );
        assert_eq!(
            code("insufficient_lots", "sell", None),
            "insufficient_holdings"
        );
        assert_eq!(
            code("non_sale_disposal_kind", "lost", None),
            "non_sale_disposal_kind"
        );
    }

    #[test]
    fn boxes_drop_the_pence() {
        let d = |t: &str| t.parse::<Decimal>().unwrap();
        assert_eq!(pounds(&d("1536.67")).unwrap(), "1536");
        // gb-05: the pool's 50,000 comes back as 49,999.999…; still 67,500.
        assert_eq!(
            pounds(&d("67499.99999999999999999999999")).unwrap(),
            "67500"
        );
        assert_eq!(money(&d("-0.0000000000001")).unwrap(), "0.00");
        assert_eq!(
            expenses_of(&d("642"), &d("562.5000000000000000000000001"), &d("79.5")).unwrap(),
            Decimal::zero()
        );
        assert_eq!(money(&d("0.125")).unwrap(), "0.12");
        assert_eq!(tax_year(Date::parse("2025-04-05").unwrap()).1, "2024-25");
    }
}
