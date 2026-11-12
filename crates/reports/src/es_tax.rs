//! Spain's country layer data: the casilla 1804 rows of every transmission,
//! the income casillas, the crypto-only integration of the savings base,
//! and the review flags, per Madrid tax year
//! (`docs/reference/tax-country-es.md`). It reads the book's journal and
//! decides nothing the journal does not already say: transmission values,
//! acquisition values and gift-loss marks are the tax engine's; this module
//! rounds, groups and totals them and places them on Modelo 100.

use super::*;
use kassiber_tax_countries::calendar::{self, Date};
use std::collections::BTreeMap;

/// What a transmission was given for (Renta WEB's tipo de contraprestación).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Sold for euros or another legal currency.
    Sale,
    /// Exchanged for another crypto asset (a permuta).
    Swap,
    /// Paid for goods or services.
    Spend,
    /// Given away (an inter vivos gratuitous transmission).
    Gift,
    /// Fee units paid to the network on a move between own wallets.
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
    /// The contraprestación the form asks for.
    pub fn contraprestacion(self) -> &'static str {
        match self {
            Kind::Sale => "moneda_curso_legal",
            Kind::Swap => "otra_moneda_virtual",
            Kind::Spend | Kind::NetworkFee => "otro_bien_o_servicio",
            Kind::Gift => "sin_contraprestacion",
        }
    }
}

/// The units of one acquisition a transmission took.
#[derive(Clone, Debug)]
pub struct Lot {
    pub transaction_id: String,
    pub quantity: Decimal,
    pub cost: Decimal,
}

/// One transmission, as the journal books it.
#[derive(Clone, Debug)]
pub struct Disposal {
    pub transaction_id: String,
    pub wallet: String,
    pub asset: String,
    /// The instant, µs since the epoch.
    pub us: i64,
    /// The Madrid date.
    pub date: Date,
    pub kind: Kind,
    /// Units transmitted, fee units of a sale included.
    pub quantity: Decimal,
    pub proceeds: Decimal,
    pub cost: Decimal,
    /// The seller's expenses (art. 35.2), deducted from the proceeds.
    pub expenses: Decimal,
    /// `false` for a gift's loss (art. 33.5.c).
    pub computable: bool,
    /// Why the proceeds are not the price (`market_value`, art. 35.2).
    pub valuation: Option<String>,
    pub lots: Vec<Lot>,
}

/// One receipt the journal books as income.
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

/// An acquisition, for the reacquisition check (art. 33.5.e).
#[derive(Clone, Debug)]
pub struct Acquisition {
    pub transaction_id: String,
    pub asset: String,
    pub us: i64,
    pub date: Date,
}

/// The casilla and base of an income kind (ES-R7), or `None` for a kind
/// without a settled treatment, which the engine refuses before it books
/// one.
pub fn casilla(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        "staking" => Some(("0027", "ahorro")),
        // A loan of own capital (art. 25.2); its casilla is open question
        // ES-Q4.
        "lending_interest" | "interest" => Some(("rcm_art25_2", "ahorro")),
        "airdrop" | "mining" | "mining_reward" => Some(("0304", "general")),
        _ => None,
    }
}

fn add(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.add(b, &DECIMAL).map_err(arithmetic)
}
fn sub(a: &Decimal, b: &Decimal) -> Result<Decimal> {
    a.sub(b, &DECIMAL).map_err(arithmetic)
}
fn min(a: &Decimal, b: &Decimal) -> Decimal {
    if a < b {
        a.clone()
    } else {
        b.clone()
    }
}

/// Half-up to the cent, as each row is rounded (ES-R14).
pub fn cents(value: &Decimal) -> Result<Decimal> {
    value
        .to_fixed_places(2, Rounding::HalfUp)
        .map_err(arithmetic)?
        .parse()
        .map_err(|_| AppError::new("internal_error", "invalid rounded amount"))
}

/// A row of the casilla 1804 annex: the transmission value (the proceeds
/// less the seller's expenses), the acquisition value and their difference,
/// each value rounded half-up to the cent first.
#[derive(Clone, Debug)]
pub struct Row {
    pub valor_transmision: Decimal,
    pub valor_adquisicion: Decimal,
    pub ganancia: Decimal,
}

pub fn row(disposal: &Disposal) -> Result<Row> {
    let valor_transmision = cents(&sub(&disposal.proceeds, &disposal.expenses)?)?;
    let valor_adquisicion = cents(&disposal.cost)?;
    let ganancia = sub(&valor_transmision, &valor_adquisicion)?;
    Ok(Row {
        valor_transmision,
        valor_adquisicion,
        ganancia,
    })
}

/// One coin and type of consideration in casilla 1804.
#[derive(Clone, Debug)]
pub struct CoinLine {
    pub asset: String,
    /// The type of consideration (`Kind::contraprestacion`).
    pub contraprestacion: &'static str,
    pub valor_transmision: Decimal,
    pub valor_adquisicion: Decimal,
    pub saldo: Decimal,
}

/// One Madrid tax year's figures.
#[derive(Clone, Debug)]
pub struct Year {
    pub year: i32,
    pub ganancias: Decimal,
    pub perdidas: Decimal,
    pub saldo: Decimal,
    pub casilla_0027: Decimal,
    pub rcm_art25_2: Decimal,
    pub casilla_0304: Decimal,
    pub non_computable_losses: Decimal,
    pub rcm_offset_25pct: Decimal,
    pub prior_losses_applied: Decimal,
    pub base: Decimal,
    /// Unexpired loss balances after the year, by year of origin; each is
    /// usable through [`CARRY_YEARS`] years after its origin.
    pub carry_forward: Vec<(i32, Decimal)>,
    pub by_coin: Vec<CoinLine>,
}

/// The years a net loss of the savings base stays usable after its year.
pub const CARRY_YEARS: i32 = 4;

/// Every Madrid year with a transmission or an income receipt, in order,
/// with the crypto-only integration of the savings base (ES-R6) carried
/// across them.
pub fn years(disposals: &[Disposal], income: &[Income]) -> Result<Vec<Year>> {
    let mut by_year: BTreeMap<i32, (Vec<&Disposal>, Vec<&Income>)> = BTreeMap::new();
    for disposal in disposals {
        by_year
            .entry(disposal.date.year)
            .or_default()
            .0
            .push(disposal);
    }
    for item in income {
        by_year.entry(item.date.year).or_default().1.push(item);
    }
    let zero = Decimal::zero;
    let quarter: Decimal = "0.25".parse().expect("a decimal");
    // Loss balances by year of origin, oldest first.
    let mut pending: Vec<(i32, Decimal)> = Vec::new();
    let mut out = Vec::new();
    for (year, (disposals, income)) in by_year {
        pending.retain(|(origin, _)| year - origin <= CARRY_YEARS);
        let (mut ganancias, mut perdidas, mut non_computable) = (zero(), zero(), zero());
        let mut coins: BTreeMap<(String, &'static str), [Decimal; 3]> = BTreeMap::new();
        for disposal in disposals {
            let row = row(disposal)?;
            if !disposal.computable {
                if row.ganancia.is_negative() {
                    non_computable = sub(&non_computable, &row.ganancia)?;
                }
                continue;
            }
            if row.ganancia.is_negative() {
                perdidas = sub(&perdidas, &row.ganancia)?;
            } else {
                ganancias = add(&ganancias, &row.ganancia)?;
            }
            let line = coins
                .entry((disposal.asset.clone(), disposal.kind.contraprestacion()))
                .or_insert_with(|| [zero(), zero(), zero()]);
            line[0] = add(&line[0], &row.valor_transmision)?;
            line[1] = add(&line[1], &row.valor_adquisicion)?;
            line[2] = add(&line[2], &row.ganancia)?;
        }
        let (mut c0027, mut rcm, mut c0304) = (zero(), zero(), zero());
        for item in income {
            let value = cents(&item.value)?;
            match casilla(&item.kind).map(|(casilla, _)| casilla) {
                Some("0027") => c0027 = add(&c0027, &value)?,
                Some("rcm_art25_2") => rcm = add(&rcm, &value)?,
                Some(_) => c0304 = add(&c0304, &value)?,
                None => {}
            }
        }
        let saldo = sub(&ganancias, &perdidas)?;
        let yields = add(&c0027, &rcm)?;
        // Up to 25 % of the year's capital-mobiliario yields may absorb
        // losses (art. 49.1.b).
        let mut capacity = if yields.is_positive() {
            yields.mul(&quarter, &DECIMAL).map_err(arithmetic)?
        } else {
            zero()
        };
        let mut gains = zero();
        let mut offset = zero();
        if saldo.is_negative() {
            // This year's net loss first offsets the 25 %; the rest carries
            // forward.
            let loss = saldo.copy_abs();
            offset = min(&loss, &capacity);
            capacity = sub(&capacity, &offset)?;
            let rest = sub(&loss, &offset)?;
            if rest.is_positive() {
                pending.push((year, rest));
            }
        } else {
            gains = saldo.clone();
        }
        // Earlier losses, oldest first: against this year's net gains, then
        // against what is left of the 25 % (art. 49.2: each as far as the
        // year allows).
        let (mut from_gains, mut from_yields) = (zero(), zero());
        for (origin, balance) in pending.iter_mut() {
            if *origin == year {
                continue;
            }
            let used = min(balance, &gains);
            gains = sub(&gains, &used)?;
            *balance = sub(balance, &used)?;
            from_gains = add(&from_gains, &used)?;
            let used = min(balance, &capacity);
            capacity = sub(&capacity, &used)?;
            *balance = sub(balance, &used)?;
            from_yields = add(&from_yields, &used)?;
        }
        pending.retain(|(_, balance)| balance.is_positive());
        let base = add(&gains, &sub(&sub(&yields, &offset)?, &from_yields)?)?;
        out.push(Year {
            year,
            ganancias,
            perdidas,
            saldo,
            casilla_0027: c0027,
            rcm_art25_2: rcm,
            casilla_0304: c0304,
            non_computable_losses: non_computable,
            rcm_offset_25pct: offset,
            prior_losses_applied: add(&from_gains, &from_yields)?,
            base,
            carry_forward: pending.clone(),
            by_coin: coins
                .into_iter()
                .map(
                    |((asset, contraprestacion), [transmision, adquisicion, saldo])| CoinLine {
                        asset,
                        contraprestacion,
                        valor_transmision: transmision,
                        valor_adquisicion: adquisicion,
                        saldo,
                    },
                )
                .collect(),
        });
    }
    Ok(out)
}

/// A review flag on a transmission, a receipt, or a year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    /// The transaction id the flag names.
    pub event: String,
    pub code: &'static str,
}

/// The days after a loss within which a reacquisition of the same coin is
/// flagged for art. 33.5.e (ES-R9: one year, leap years included).
pub const REACQUISITION_DAYS: i64 = 366;

/// The flags the report lists for review, in the order of the rows they
/// name: a gift's non-computable loss, a price replaced by market value, a
/// loss followed by a reacquisition within a year, lending income without
/// a confirmed casilla.
pub fn flags(
    disposals: &[Disposal],
    income: &[Income],
    acquisitions: &[Acquisition],
) -> Result<Vec<Flag>> {
    let mut flags = Vec::new();
    for disposal in disposals {
        let row = row(disposal)?;
        let flag = |code| Flag {
            event: disposal.transaction_id.clone(),
            code,
        };
        if !disposal.computable && row.ganancia.is_negative() {
            flags.push(flag("es_gift_loss_not_computable"));
        }
        if disposal.valuation.as_deref() == Some("market_value") {
            flags.push(flag("es_price_below_market"));
        }
        if disposal.computable && row.ganancia.is_negative() {
            let reacquired = acquisitions.iter().any(|acquisition| {
                acquisition.asset == disposal.asset
                    && acquisition.us > disposal.us
                    && acquisition.date.to_days() - disposal.date.to_days() <= REACQUISITION_DAYS
            });
            if reacquired {
                flags.push(flag("es_33_5_e_reacquisition_review"));
            }
        }
    }
    for item in income {
        if casilla(&item.kind).is_some_and(|(casilla, _)| casilla == "rcm_art25_2") {
            flags.push(Flag {
                event: item.transaction_id.clone(),
                code: "es_lending_casilla_unconfirmed",
            });
        }
    }
    Ok(flags)
}

/// Money in the vectors' form: cents, half-up.
pub fn money(value: &Decimal) -> Result<String> {
    value
        .to_fixed_places(2, Rounding::HalfUp)
        .map_err(arithmetic)
}

/// One year's figures in the shared vector schema's `years` form
/// (`docs/reference/tax-country-es-vectors.json`).
pub fn year_json(year: &Year) -> Result<serde_json::Value> {
    let mut carry = serde_json::Map::new();
    for (origin, balance) in &year.carry_forward {
        carry.insert(origin.to_string(), money(balance)?.into());
    }
    Ok(serde_json::json!({
        "casilla_1804": {
            "ganancias": money(&year.ganancias)?,
            "perdidas": money(&year.perdidas)?,
            "saldo": money(&year.saldo)?,
        },
        "casilla_0027": money(&year.casilla_0027)?,
        "rcm_art25_2": money(&year.rcm_art25_2)?,
        "casilla_0304": money(&year.casilla_0304)?,
        "non_computable_losses": money(&year.non_computable_losses)?,
        "ahorro_crypto_only": {
            "rcm_offset_25pct": money(&year.rcm_offset_25pct)?,
            "prior_losses_applied": money(&year.prior_losses_applied)?,
            "base": money(&year.base)?,
            "carry_forward": carry,
        },
    }))
}

/// The instant (µs) of a stored timestamp, and its date in `zone`.
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

/// An outbound kind that pays for goods or services rather than selling.
const SPEND_TOKENS: [&str; 3] = ["spend", "payment", "purchase"];

/// The book's transmissions, receipts and acquisitions, from its journal,
/// in date order. A `wallet` id limits them to that wallet.
pub fn load(
    conn: &Connection,
    profile_id: &str,
    wallet: Option<&str>,
) -> Result<(Vec<Disposal>, Vec<Income>, Vec<Acquisition>)> {
    let zone = calendar::zone(kassiber_tax_countries::MADRID).expect("Madrid has a calendar");
    let swaps: std::collections::HashSet<String> = rows(
        conn,
        "SELECT out_transaction_id FROM transaction_pairs \
         WHERE profile_id=?1 AND deleted_at IS NULL AND policy='taxable'",
        [profile_id],
    )?
    .iter()
    .map(|row| text(row, "out_transaction_id"))
    .collect();
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
    let (mut disposals, mut income, mut acquisitions) = (Vec::new(), Vec::new(), Vec::new());
    for entry in &entries {
        let (us, date) = instant(&text(entry, "occurred_at"), &zone)?;
        let transaction_id = text(entry, "transaction_id");
        let asset = text(entry, "asset");
        let quantity = btc(int(entry, "quantity"))?.copy_abs();
        let kind = kassiber_pytext::lower(kassiber_pytext::strip(&text(entry, "kind")));
        match text(entry, "entry_type").as_str() {
            "acquisition" => acquisitions.push(Acquisition {
                transaction_id,
                asset,
                us,
                date,
            }),
            "income" => income.push(Income {
                transaction_id,
                wallet: text(entry, "wallet"),
                asset,
                us,
                date,
                kind,
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
                let mut lots = Vec::new();
                // Each fragment stores its share of the entry's expenses.
                let mut expenses = Decimal::zero();
                if let Some(calculation) = &calculation {
                    if let Value::List(fragments) = value(calculation, "fragments") {
                        for fragment in fragments {
                            if truthy(value(fragment, "expenses_exact")) {
                                expenses =
                                    add(&expenses, &decimal(value(fragment, "expenses_exact"))?)?;
                            }
                            let lot = value(fragment, "lot");
                            lots.push(Lot {
                                transaction_id: text(lot, "transaction_id"),
                                quantity: btc(int(fragment, "quantity_msat"))?.copy_abs(),
                                cost: decimal(value(fragment, "cost_basis_exact"))?,
                            });
                        }
                    }
                }
                let kind = if entry_type != "disposal" {
                    Kind::NetworkFee
                } else if kind.contains("gift") {
                    Kind::Gift
                } else if swaps.contains(&transaction_id) {
                    Kind::Swap
                } else if SPEND_TOKENS.iter().any(|token| kind.contains(token)) {
                    Kind::Spend
                } else {
                    Kind::Sale
                };
                disposals.push(Disposal {
                    transaction_id,
                    wallet: text(entry, "wallet"),
                    asset,
                    us,
                    date,
                    kind,
                    quantity,
                    proceeds: exact(entry, "proceeds")?,
                    cost: exact(entry, "cost_basis")?,
                    expenses,
                    computable: !matches!(fact("loss_deductible"), Value::Bool(false)),
                    valuation: truthy(fact("valuation")).then(|| py_str(fact("valuation"))),
                    lots,
                });
            }
        }
    }
    // Entries of one instant are stored in no particular order.
    disposals.sort_by(|a, b| (a.us, &a.transaction_id).cmp(&(b.us, &b.transaction_id)));
    income.sort_by(|a, b| (a.us, &a.transaction_id).cmp(&(b.us, &b.transaction_id)));
    acquisitions.sort_by(|a, b| (a.us, &a.transaction_id).cmp(&(b.us, &b.transaction_id)));
    Ok((disposals, income, acquisitions))
}

fn dec(d: &Decimal) -> Value {
    s(d.to_string())
}

/// The layer's part of a report model: rows, receipts, years and flags,
/// exact and unformatted.
pub fn model(conn: &Connection, profile: &Value, wallet: Option<&str>) -> Result<Value> {
    let (disposals, income, acquisitions) = load(conn, &text(profile, "id"), wallet)?;
    let years = years(&disposals, &income)?;
    let flags = flags(&disposals, &income, &acquisitions)?;
    let mut rows = Vec::new();
    for d in &disposals {
        let row = row(d)?;
        rows.push(dict([
            ("transaction_id", s(d.transaction_id.as_str())),
            ("wallet", s(d.wallet.as_str())),
            ("asset", s(d.asset.as_str())),
            ("year", Value::Int(d.date.year.into())),
            ("date", s(d.date.iso())),
            ("kind", s(d.kind.code())),
            ("contraprestacion", s(d.kind.contraprestacion())),
            ("quantity", dec(&d.quantity)),
            ("valor_transmision", dec(&row.valor_transmision)),
            ("valor_adquisicion", dec(&row.valor_adquisicion)),
            ("ganancia", dec(&row.ganancia)),
            ("computable", Value::Bool(d.computable)),
        ]));
    }
    let receipts = income
        .iter()
        .map(|i| {
            let (casilla, base) = casilla(&i.kind).unwrap_or(("", ""));
            dict([
                ("transaction_id", s(i.transaction_id.as_str())),
                ("wallet", s(i.wallet.as_str())),
                ("asset", s(i.asset.as_str())),
                ("date", s(i.date.iso())),
                ("kind", s(i.kind.as_str())),
                ("quantity", dec(&i.quantity)),
                ("value", dec(&i.value)),
                ("casilla", s(casilla)),
                ("base", s(base)),
            ])
        })
        .collect();
    let mut year_values = Vec::new();
    for y in &years {
        year_values.push(dict([
            ("year", Value::Int(y.year.into())),
            ("ganancias", dec(&y.ganancias)),
            ("perdidas", dec(&y.perdidas)),
            ("saldo", dec(&y.saldo)),
            ("non_computable_losses", dec(&y.non_computable_losses)),
            ("casilla_0027", dec(&y.casilla_0027)),
            ("rcm_art25_2", dec(&y.rcm_art25_2)),
            ("rcm", dec(&add(&y.casilla_0027, &y.rcm_art25_2)?)),
            ("casilla_0304", dec(&y.casilla_0304)),
            ("rcm_offset_25pct", dec(&y.rcm_offset_25pct)),
            ("prior_losses_applied", dec(&y.prior_losses_applied)),
            ("base", dec(&y.base)),
            (
                "carry_forward",
                Value::List(
                    y.carry_forward
                        .iter()
                        .map(|(origin, amount)| {
                            dict([
                                ("origin", Value::Int((*origin).into())),
                                ("amount", dec(amount)),
                                ("last_year", Value::Int((origin + CARRY_YEARS).into())),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "by_coin",
                Value::List(
                    y.by_coin
                        .iter()
                        .map(|line| {
                            dict([
                                ("asset", s(line.asset.as_str())),
                                ("contraprestacion", s(line.contraprestacion)),
                                ("valor_transmision", dec(&line.valor_transmision)),
                                ("valor_adquisicion", dec(&line.valor_adquisicion)),
                                ("saldo", dec(&line.saldo)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]));
    }
    Ok(dict([
        ("country", s("es")),
        ("wallet_scoped", Value::Bool(wallet.is_some())),
        ("disposals", Value::List(rows)),
        ("income", Value::List(receipts)),
        ("years", Value::List(year_values)),
        (
            "flags",
            Value::List(
                flags
                    .iter()
                    .map(|flag| dict([("event", s(flag.event.as_str())), ("code", s(flag.code))]))
                    .collect(),
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spanish_entry_is_dated_in_madrid() {
        // es-14: 23:30 UTC on 31 December is 00:30 on 1 January in Madrid,
        // and a summer evening stays on its day.
        let zone = calendar::zone(kassiber_tax_countries::MADRID).unwrap();
        let (us, date) = instant("2025-12-31T23:30:00Z", &zone).unwrap();
        assert_eq!(date.iso(), "2026-01-01");
        assert_eq!(us, 1_767_223_800_000_000);
        let (_, date) = instant("2025-06-30T21:59:59Z", &zone).unwrap();
        assert_eq!(date.iso(), "2025-06-30");
        assert!(instant("bad", &zone).is_err());
    }
}
