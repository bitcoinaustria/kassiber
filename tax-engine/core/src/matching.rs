//! Country matching methods: a method that computes an asset's fragments
//! as a whole instead of one lot seek at a time.
//!
//! A lot method ([`crate::methods`]) answers "which lot next" for one
//! disposal. Some countries' methods cannot: the United Kingdom matches a
//! day's disposals with the same day's acquisitions, then the next 30
//! days', then its section 104 pool; Japan's total average prices every
//! disposal of a year at the year's average; Germany's
//! `moving_average_fifo_period` takes FIFO lots for the holding period but
//! values them at the wallet's moving average. Such a method implements
//! [`Matching`] and is listed in [`method`]; `compute_parsed` hands it the
//! prepared asset (every entry, the visible lots, the taxable events in
//! order), the country (its zone and calendar) and the pool scope, and
//! derives the outputs from its fragments as for any method. A fragment
//! names a lot, whose acquisition dates its holding period, and may carry
//! a per-unit basis that replaces the lot's (`unit_cost_basis_override`).

use kassiber_pydecimal::{Context, Rounding};

use crate::cursor::{Fragment, LOTS_EXHAUSTED};
use crate::decimal::Decimal;
use crate::engine::{Country, PreparedAsset};
use crate::error::{EngineError, EngineResult};
use crate::model::{MethodName, PoolScope, WalletHoldingOut};
use crate::num::{add, div, mul, sub, zero};
use crate::wallets::{self, Basis, Order};

/// What a matching method computed for one asset.
pub(crate) struct Matched {
    /// In the asset's taxable-event order.
    pub fragments: Vec<Fragment>,
    /// Each wallet's remaining units and basis, under the wallet scope.
    pub wallet_holdings: Option<Vec<WalletHoldingOut>>,
    /// Each lot's full-quantity basis for its open position, when the
    /// method's remaining cost is not the lots' own (a pool's average);
    /// the lots' own basis when `None`.
    pub open_basis: Option<Vec<Decimal>>,
}

/// A country method that computes an asset as a whole.
pub(crate) trait Matching: Sync {
    fn compute(
        &self,
        asset: &PreparedAsset,
        country: Country,
        scope: PoolScope,
    ) -> EngineResult<Matched>;
}

/// The matching method `method` names, if it is one.
pub(crate) fn method(method: MethodName) -> Option<&'static dyn Matching> {
    match method {
        MethodName::MovingAverageFifoPeriod => Some(&MovingAverageFifoPeriod),
        MethodName::UkShareMatching => Some(&crate::share_matching::UkShareMatching),
        MethodName::JpTotalAverage => Some(&JpAverage { total: true }),
        MethodName::JpMovingAverage => Some(&JpAverage { total: false }),
        MethodName::Fifo
        | MethodName::Lifo
        | MethodName::Hifo
        | MethodName::Lofo
        | MethodName::MovingAverage
        | MethodName::MovingAverageAt => None,
    }
}

/// Germany's main rule (BMF 6 March 2025 Rn. 61): the holding period by
/// FIFO, the value by the average method, per wallet (Rn. 62). Each
/// disposal takes the wallet's FIFO portions, so their acquisition dates
/// decide what is within the year, at the wallet's moving average cost per
/// unit: acquisitions raise it, disposals leave it, and a move carries the
/// source's average to its destination (`docs/reference/tax-country-de.md`,
/// DE-METHOD-AVG, vector de-19).
struct MovingAverageFifoPeriod;

impl Matching for MovingAverageFifoPeriod {
    fn compute(
        &self,
        asset: &PreparedAsset,
        _country: Country,
        scope: PoolScope,
    ) -> EngineResult<Matched> {
        if scope != PoolScope::Wallet {
            return Err(EngineError::value(
                "Accounting method 'moving_average_fifo_period' averages per wallet: choose the wallet pool scope",
            ));
        }
        let (fragments, holdings) = wallets::fragments_with(
            &asset.entries,
            &asset.lots,
            &asset.ins,
            &asset.events,
            Order::Fifo,
            Basis::WalletAverage,
        )?;
        Ok(Matched {
            fragments,
            wallet_holdings: Some(holdings),
            open_basis: None,
        })
    }
}

/// Japan's averages (所得税法施行令 119条の2), over every wallet of the book
/// (`docs/reference/tax-country-jp.md`, JP-R3 to JP-R5, JP-R12):
///
/// - total average (総平均法, the statutory default): every disposal of a tax
///   year costs `(opening value + the year's acquisitions) × quantity ÷
///   (opening quantity + the year's quantity)`, one 32-digit step each, and
///   the year's closing value `total − costs` carries forward unrounded;
/// - moving average (移動平均法): each acquisition revises the unit cost to
///   `(book value + cost) ÷ quantity`, and a disposal leaves the book value
///   at `ROUNDUP(unit × quantity left, 0)` (the NTA calculation sheet), its
///   cost being the drop.
///
/// Fragments take FIFO portions of the lots, which no Japanese rule reads
/// but open positions and acquisition dates need, each carrying its share
/// of the disposal's cost (the last portion the remainder, so a disposal's
/// fragments add up to its cost exactly).
struct JpAverage {
    total: bool,
}

/// The book value left after a moving-average disposal: up to the yen.
fn round_up_yen(value: &Decimal) -> EngineResult<Decimal> {
    Ok(value.quantize(0, &Context::new(32, Rounding::Ceiling))?)
}

impl Matching for JpAverage {
    fn compute(
        &self,
        asset: &PreparedAsset,
        country: Country,
        scope: PoolScope,
    ) -> EngineResult<Matched> {
        let name = if self.total {
            "jp_total_average"
        } else {
            "jp_moving_average"
        };
        if scope != PoolScope::Global {
            return Err(EngineError::value(format!(
                "Accounting method '{name}' averages over every wallet: choose the global pool scope"
            )));
        }
        let Country::Rules(rules) = country else {
            return Err(EngineError::value(format!(
                "Accounting method '{name}' needs a registry country's calendar"
            )));
        };
        let entries = &asset.entries;
        let lots = &asset.lots;
        let year = |index: usize| -> EngineResult<i32> {
            Ok(kassiber_tax_countries::years::tax_year(
                rules.local_date(&entries[index].ts)?,
                rules.tax_year_start,
            ))
        };
        // Each disposal's cost, by its position in the events.
        let disposals: Vec<usize> = (0..asset.events.len())
            .filter(|&at| !entries[asset.events[at]].is_earning())
            .collect();
        let mut costs: Vec<Option<Decimal>> = vec![None; asset.events.len()];
        let (closing_quantity, closing_value) = if self.total {
            // Per tax year: the year's acquisitions and disposals.
            let mut years: std::collections::BTreeMap<i32, ([Decimal; 2], Vec<usize>)> =
                std::collections::BTreeMap::new();
            for lot in lots.iter() {
                let slot = years
                    .entry(year(lot.entry)?)
                    .or_insert_with(|| ([zero(), zero()], Vec::new()));
                slot.0[0] = add(&slot.0[0], &lot.crypto_in)?;
                slot.0[1] = add(&slot.0[1], &lot.fiat_in_with_fee)?;
            }
            for &at in &disposals {
                years
                    .entry(year(asset.events[at])?)
                    .or_insert_with(|| ([zero(), zero()], Vec::new()))
                    .1
                    .push(at);
            }
            let (mut quantity, mut value) = (zero(), zero());
            for ([acquired, acquired_value], year_disposals) in years.into_values() {
                let total_quantity = add(&quantity, &acquired)?;
                let total_value = add(&value, &acquired_value)?;
                let (mut disposed, mut cost_sum) = (zero(), zero());
                for at in year_disposals {
                    let amount = entries[asset.events[at]].crypto_balance_change();
                    if !total_quantity.is_positive() {
                        return Err(EngineError::value(LOTS_EXHAUSTED));
                    }
                    let cost = div(&mul(&total_value, amount)?, &total_quantity)?;
                    disposed = add(&disposed, amount)?;
                    cost_sum = add(&cost_sum, &cost)?;
                    costs[at] = Some(cost);
                }
                quantity = sub(&total_quantity, &disposed)?;
                value = sub(&total_value, &cost_sum)?;
                if quantity.is_negative() {
                    return Err(EngineError::value(LOTS_EXHAUSTED));
                }
            }
            (quantity, value)
        } else {
            // Acquisitions before disposals at one instant, as the walk orders
            // them.
            let mut timeline: Vec<(i64, u8, usize)> = lots
                .iter()
                .enumerate()
                .map(|(position, lot)| (lot.us, 0, position))
                .chain(
                    disposals
                        .iter()
                        .map(|&at| (entries[asset.events[at]].ts.us, 1, at)),
                )
                .collect();
            timeline.sort();
            let (mut quantity, mut book, mut unit) = (zero(), zero(), zero());
            for (_, kind, index) in timeline {
                if kind == 0 {
                    let lot = &lots[index];
                    quantity = add(&quantity, &lot.crypto_in)?;
                    book = add(&book, &lot.fiat_in_with_fee)?;
                    unit = div(&book, &quantity)?;
                } else {
                    let amount = entries[asset.events[index]].crypto_balance_change();
                    let left = sub(&quantity, amount)?;
                    if left.is_negative() {
                        return Err(EngineError::value(LOTS_EXHAUSTED));
                    }
                    let kept = round_up_yen(&mul(&unit, &left)?)?;
                    costs[index] = Some(sub(&book, &kept)?);
                    book = kept;
                    quantity = left;
                }
            }
            (quantity, book)
        };
        // FIFO portions of the lots, in event order.
        let mut remaining: Vec<Decimal> = lots.iter().map(|lot| lot.crypto_in.clone()).collect();
        let mut head = 0;
        let mut fragments = Vec::new();
        for (at, &event) in asset.events.iter().enumerate() {
            let entry = &entries[event];
            if entry.is_earning() {
                fragments.push(Fragment {
                    event,
                    lot: None,
                    crypto_amount: entry.crypto_balance_change().clone(),
                    unit_cost_basis_override: None,
                    cost_override: None,
                });
                continue;
            }
            let amount = entry.crypto_balance_change();
            let cost = costs[at].clone().expect("every disposal has its cost");
            let mut left = amount.clone();
            let mut assigned = zero();
            while left.is_positive() {
                while head < lots.len() && !remaining[head].is_positive() {
                    head += 1;
                }
                if head == lots.len() || lots[head].us > entry.ts.us {
                    return Err(EngineError::value(LOTS_EXHAUSTED));
                }
                let take = if remaining[head] < left {
                    remaining[head].clone()
                } else {
                    left.clone()
                };
                remaining[head] = sub(&remaining[head], &take)?;
                left = sub(&left, &take)?;
                let share = if left.is_positive() {
                    div(&mul(&cost, &take)?, amount)?
                } else {
                    sub(&cost, &assigned)?
                };
                assigned = add(&assigned, &share)?;
                fragments.push(Fragment {
                    event,
                    lot: Some(head),
                    crypto_amount: take,
                    unit_cost_basis_override: None,
                    cost_override: Some(share),
                });
            }
        }
        // The units left are worth the closing value: each open lot at the
        // closing unit cost.
        let unit = if closing_quantity.is_positive() {
            div(&closing_value, &closing_quantity)?
        } else {
            zero()
        };
        let open_basis = lots
            .iter()
            .map(|lot| mul(&unit, &lot.crypto_in))
            .collect::<EngineResult<Vec<_>>>()?;
        Ok(Matched {
            fragments,
            wallet_holdings: None,
            open_basis: Some(open_basis),
        })
    }
}
