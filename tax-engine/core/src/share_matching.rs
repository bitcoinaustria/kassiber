//! The United Kingdom's share matching for cryptoassets (CRYPTO22200;
//! `docs/reference/tax-country-gb.md`): per token and per UK calendar day,
//! all of a day's acquisitions are one acquisition and all of its disposals
//! one disposal, matched in this order:
//!
//! 1. the same day's acquisitions (TCGA92/S105), at their average cost;
//! 2. acquisitions in the 30 days after the disposal day (S106A(5)),
//!    earlier disposals first and, within the window, earlier acquisition
//!    days first;
//! 3. the section 104 pool (S104) at its average cost, which every
//!    acquisition's unmatched remainder joins on its day.
//!
//! The method needs the whole asset, so it is a [`Matching`] method. Each
//! match becomes fragments of the day's disposal events in their order:
//! a same-day or 30-day match names the acquisition day's lots in lot
//! order, a pool match the pool's lots in the order they joined it, and
//! every fragment carries its match's cost per unit
//! (`unit_cost_basis_override`). A 30-day match names a lot acquired after
//! its disposal, which the lot methods never do. The pool's remaining cost
//! is the open positions' basis, so holdings read the pool's average.
//!
//! A disposal the pool cannot cover fails rather than take units it does
//! not hold; the 30-day rule makes a day's figures final only once 30 days
//! have passed, which the country's report states.

use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};

use crate::cursor::{Fragment, LOTS_EXHAUSTED};
use crate::decimal::Decimal;
use crate::engine::{Country, PreparedAsset};
use crate::error::{EngineError, EngineResult};
use crate::matching::{Matched, Matching};
use crate::model::PoolScope;
use crate::num::{add, cmp13, div, gt13, mul, sub, zero};
use crate::time::CivilDate;

/// The 30-day rule's window: the 30 calendar days after the disposal day.
const WINDOW_DAYS: i64 = 30;

pub(crate) struct UkShareMatching;

/// One day's acquisitions: its lots in lot order with what each has left
/// for matching, and the day's remaining quantity and cost.
struct Acquired {
    lots: Vec<(usize, Decimal)>,
    quantity: Decimal,
    cost: Decimal,
}

/// One day's disposal events in their order, the day's remaining quantity,
/// and the pieces matched to it so far: `(lot, quantity, cost per unit)`.
struct Disposed {
    events: Vec<usize>,
    remaining: Decimal,
    pieces: Vec<(usize, Decimal, Decimal)>,
}

impl Acquired {
    /// Takes `quantity` of the day's remainder at its average cost, lot by
    /// lot in lot order, as pieces at that cost per unit.
    fn take(&mut self, quantity: &Decimal) -> EngineResult<Vec<(usize, Decimal, Decimal)>> {
        let cost = div(&mul(&self.cost, quantity)?, &self.quantity)?;
        let unit = div(&cost, quantity)?;
        self.cost = sub(&self.cost, &cost)?;
        self.quantity = sub(&self.quantity, quantity)?;
        Ok(take_lots(&mut self.lots, quantity, &unit))
    }
}

/// Takes `quantity` from the front of `lots`, each piece at `unit`.
fn take_lots(
    lots: &mut [(usize, Decimal)],
    quantity: &Decimal,
    unit: &Decimal,
) -> Vec<(usize, Decimal, Decimal)> {
    let mut needed = quantity.clone();
    let mut pieces = Vec::new();
    for (lot, left) in lots.iter_mut() {
        if needed.is_zero() {
            break;
        }
        if left.is_zero() {
            continue;
        }
        // Quantities are exact (msat), so the comparison is exact too.
        let piece = if *left > needed {
            needed.clone()
        } else {
            left.clone()
        };
        *left = left.sub(&piece, &crate::num::CTX).expect("exact");
        needed = needed.sub(&piece, &crate::num::CTX).expect("exact");
        pieces.push((*lot, piece, unit.clone()));
    }
    pieces
}

/// The minimum of two exact quantities.
fn min(a: &Decimal, b: &Decimal) -> Decimal {
    if a <= b {
        a.clone()
    } else {
        b.clone()
    }
}

impl Matching for UkShareMatching {
    fn compute(
        &self,
        asset: &PreparedAsset,
        country: Country,
        scope: PoolScope,
    ) -> EngineResult<Matched> {
        let Country::Rules(rules) = country else {
            return Err(EngineError::value(
                "Accounting method 'uk_share_matching' needs a country calendar: it matches by local day",
            ));
        };
        if scope != PoolScope::Global {
            return Err(EngineError::value(
                "Accounting method 'uk_share_matching' matches one pool per token: choose the global pool scope",
            ));
        }
        let entries = &asset.entries;
        let day_of = |us_entry: usize| -> EngineResult<CivilDate> {
            rules.local_date(&entries[us_entry].ts)
        };

        let mut acquired: BTreeMap<CivilDate, Acquired> = BTreeMap::new();
        for (lot, view) in asset.lots.iter().enumerate() {
            let day = acquired
                .entry(day_of(view.entry)?)
                .or_insert_with(|| Acquired {
                    lots: Vec::new(),
                    quantity: zero(),
                    cost: zero(),
                });
            day.lots.push((lot, view.crypto_in.clone()));
            day.quantity = add(&day.quantity, &view.crypto_in)?;
            day.cost = add(&day.cost, &view.fiat_in_with_fee)?;
        }
        let mut disposed: BTreeMap<CivilDate, Disposed> = BTreeMap::new();
        for &index in &asset.events {
            let event = &entries[index];
            if event.is_earning() || event.crypto_balance_change().is_zero() {
                continue;
            }
            let day = disposed.entry(day_of(index)?).or_insert_with(|| Disposed {
                events: Vec::new(),
                remaining: zero(),
                pieces: Vec::new(),
            });
            day.events.push(index);
            day.remaining = add(&day.remaining, event.crypto_balance_change())?;
        }

        // 1. The same day.
        for (day, disposal) in disposed.iter_mut() {
            if let Some(acquisition) = acquired.get_mut(day) {
                let matched = min(&acquisition.quantity, &disposal.remaining);
                if matched.is_positive() {
                    disposal.pieces.extend(acquisition.take(&matched)?);
                    disposal.remaining = sub(&disposal.remaining, &matched)?;
                }
            }
        }
        // 2. The 30 days after, earlier disposals first.
        for (day, disposal) in disposed.iter_mut() {
            let first = CivilDate::from_days(day.to_days() + 1);
            let last = CivilDate::from_days(day.to_days() + WINDOW_DAYS);
            for (_, acquisition) in acquired.range_mut(first..=last) {
                if !disposal.remaining.is_positive() {
                    break;
                }
                let matched = min(&acquisition.quantity, &disposal.remaining);
                if matched.is_positive() {
                    disposal.pieces.extend(acquisition.take(&matched)?);
                    disposal.remaining = sub(&disposal.remaining, &matched)?;
                }
            }
        }
        // 3. The section 104 pool, day by day: a day's unmatched
        // acquisitions join it before the day's disposals take from it.
        let mut pool_quantity = zero();
        let mut pool_cost = zero();
        let mut pool: VecDeque<(usize, Decimal)> = VecDeque::new();
        let mut days: Vec<CivilDate> = acquired.keys().chain(disposed.keys()).copied().collect();
        days.sort_unstable();
        days.dedup();
        for day in days {
            if let Some(acquisition) = acquired.get_mut(&day) {
                pool_quantity = add(&pool_quantity, &acquisition.quantity)?;
                pool_cost = add(&pool_cost, &acquisition.cost)?;
                pool.extend(
                    acquisition
                        .lots
                        .iter()
                        .filter(|(_, left)| left.is_positive())
                        .cloned(),
                );
            }
            let Some(disposal) = disposed.get_mut(&day) else {
                continue;
            };
            if !disposal.remaining.is_positive() {
                continue;
            }
            let needed = disposal.remaining.clone();
            if gt13(&needed, &pool_quantity)? {
                let event = &entries[disposal.events[0]];
                return Err(EngineError::value(format!(
                    "{LOTS_EXHAUSTED}: the section 104 pool holds {pool_quantity} on {}, the day's disposals need {needed} more: {}",
                    day.iso(),
                    event.text
                )));
            }
            let cost = div(&mul(&pool_cost, &needed)?, &pool_quantity)?;
            let unit = div(&cost, &needed)?;
            pool_cost = sub(&pool_cost, &cost)?;
            pool_quantity = sub(&pool_quantity, &needed)?;
            let lots = pool.make_contiguous();
            disposal.pieces.extend(take_lots(lots, &needed, &unit));
            pool.retain(|(_, left)| left.is_positive());
            disposal.remaining = zero();
        }

        // Each day's pieces become its events' fragments, in event order.
        let mut by_event: BTreeMap<usize, Vec<Fragment>> = BTreeMap::new();
        for disposal in disposed.into_values() {
            let mut pieces = disposal.pieces.into_iter();
            let mut current: Option<(usize, Decimal, Decimal)> = None;
            for index in disposal.events {
                let mut needed = entries[index].crypto_balance_change().clone();
                let out = by_event.entry(index).or_default();
                while needed.is_positive() {
                    let (lot, left, unit) = match current.take() {
                        Some(piece) => piece,
                        None => pieces.next().ok_or_else(|| {
                            EngineError::runtime(
                                "Internal error: share matching left a disposal unmatched",
                            )
                        })?,
                    };
                    let amount = min(&left, &needed);
                    check(
                        &amount,
                        entries[index].crypto_balance_change(),
                        &asset.lots[lot].crypto_in,
                        &unit,
                    )?;
                    out.push(Fragment {
                        event: index,
                        lot: Some(lot),
                        crypto_amount: amount.clone(),
                        unit_cost_basis_override: Some(unit.clone()),
                        cost_override: None,
                    });
                    needed = sub(&needed, &amount)?;
                    let rest = sub(&left, &amount)?;
                    if rest.is_positive() {
                        current = Some((lot, rest, unit));
                    }
                }
            }
        }
        let mut fragments = Vec::new();
        for &index in &asset.events {
            let event = &entries[index];
            if event.is_earning() {
                fragments.push(Fragment {
                    event: index,
                    lot: None,
                    crypto_amount: event.crypto_balance_change().clone(),
                    unit_cost_basis_override: None,
                    cost_override: None,
                });
            } else {
                fragments.extend(by_event.remove(&index).unwrap_or_default());
            }
        }

        // The lots still in the pool carry its average cost.
        let open_basis = if pool_quantity.is_positive() {
            let unit = div(&pool_cost, &pool_quantity)?;
            Some(
                asset
                    .lots
                    .iter()
                    .map(|lot| mul(&lot.crypto_in, &unit))
                    .collect::<EngineResult<Vec<_>>>()?,
            )
        } else {
            None
        };
        Ok(Matched {
            fragments,
            wallet_holdings: None,
            open_basis,
        })
    }
}

/// The checks `GainLoss`'s constructor runs on a fragment, but for its
/// event-before-lot check: a 30-day match takes a later acquisition.
fn check(
    amount: &Decimal,
    change: &Decimal,
    crypto_in: &Decimal,
    unit: &Decimal,
) -> EngineResult<()> {
    if !amount.is_positive() || unit.is_negative() {
        return Err(EngineError::value(format!(
            "Parameter 'crypto_amount' has non-positive value {amount}"
        )));
    }
    if cmp13(amount, change)? == Ordering::Greater || cmp13(amount, crypto_in)? == Ordering::Greater
    {
        return Err(EngineError::value(format!(
            "crypto_amount ({amount}) is greater than taxable event amount ({change}) or acquired-lot amount"
        )));
    }
    Ok(())
}
