//! Pool methods: the generic moving average and the Austrian
//! `moving_average_at`.
//!
//! Both pair lots in FIFO order for the audit trail and report the running
//! pool average as each fragment's unit cost basis. The average is
//! recomputed at every seek, so its last digit can drift between fragments
//! of one disposal, as RP2's does. This ports the owner-authored
//! `moving_average.py` and `moving_average_at.py`.

use std::collections::{BTreeMap, BTreeSet};

use crate::austria::{
    has_swap_link, lot_regime, pool_id, py_repr, py_str_list, regime_from_notes, swap_link_id,
    Regime,
};
use crate::decimal::Decimal;
use crate::entry::{Entry, TransactionType};
use crate::error::{EngineError, EngineResult};
use crate::methods::{Fifo, LotMethod, LotState, LotView, Selection};
use crate::num::{add, div, gt13, lt13, mul, sub, zero};

/// One pool of RP2's `PoolAcquiredLotCandidates`: its quantity, its cost,
/// and the last lot synced into it.
#[derive(Clone, Debug)]
struct Pool {
    qty: Decimal,
    cost: Decimal,
    /// `None` stands for RP2's `-1`: no lot synced yet.
    last_synced: Option<usize>,
}

impl Default for Pool {
    fn default() -> Self {
        Pool {
            qty: zero(),
            cost: zero(),
            last_synced: None,
        }
    }
}

/// The highest lot index a seek with eligibility bound `bound` considers.
fn upper_index(lots: &[LotView], bound: usize) -> Option<usize> {
    lots.len().checked_sub(1).map(|last| bound.min(last))
}

impl Pool {
    /// `cost ÷ qty` while the quantity is positive at 13 places, else `0`.
    fn average(&self) -> EngineResult<Decimal> {
        if gt13(&self.qty, &zero())? {
            div(&self.cost, &self.qty)
        } else {
            Ok(zero())
        }
    }

    /// `deduct_from_pool`: `qty − amount`, then `cost − amount × average`.
    fn deduct(&mut self, amount: &Decimal, average: &Decimal) -> EngineResult<()> {
        self.qty = sub(&self.qty, amount)?;
        self.cost = sub(&self.cost, &mul(amount, average)?)?;
        Ok(())
    }

    /// Adds each not yet synced lot up to `bound` that `include` accepts:
    /// its available amount `r` and `(basis × r) ÷ crypto_in`.
    fn sync(
        &mut self,
        lots: &[LotView],
        state: &LotState,
        bound: usize,
        mut include: impl FnMut(usize) -> EngineResult<bool>,
    ) -> EngineResult<()> {
        let Some(upper) = upper_index(lots, bound) else {
            return Ok(());
        };
        let start = self.last_synced.map_or(0, |synced| synced + 1);
        for lot in start..=upper {
            if !include(lot)? {
                continue;
            }
            let view = &lots[lot];
            let remaining = state.partial[lot].as_ref().unwrap_or(&view.crypto_in);
            self.qty = add(&self.qty, remaining)?;
            let share = div(&mul(state.basis(lots, lot), remaining)?, &view.crypto_in)?;
            self.cost = add(&self.cost, &share)?;
        }
        self.last_synced = Some(upper);
        Ok(())
    }
}

/// The amount a fragment takes: the remaining need, or the lot's available
/// amount when that is not larger at 13 places.
fn consumed(needed: &Decimal, available: &Decimal) -> EngineResult<Decimal> {
    Ok(if lt13(needed, available)? {
        needed.clone()
    } else {
        available.clone()
    })
}

/// The generic moving average: one pool (RP2's `"default"`), markers
/// ignored.
#[derive(Debug, Default)]
pub(crate) struct MovingAverage {
    fifo: Fifo,
    pool: Pool,
}

impl LotMethod for MovingAverage {
    fn seek(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
        needed: &Decimal,
        event: &Entry,
    ) -> EngineResult<Option<Selection>> {
        self.pool.sync(lots, state, bound, |_| Ok(true))?;
        // FIFO picks the lot; its own per-unit override is discarded.
        let Some(selection) = self.fifo.seek(lots, state, bound, needed, event)? else {
            return Ok(None);
        };
        let average = self.pool.average()?;
        self.pool
            .deduct(&consumed(needed, &selection.amount)?, &average)?;
        Ok(Some(Selection {
            unit_cost_basis_override: Some(average),
            taxable_event_unit_cost_basis: None,
            ..selection
        }))
    }

    fn open_position_basis(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
    ) -> EngineResult<Vec<Decimal>> {
        self.pool.sync(lots, state, bound, |_| Ok(true))?;
        let average = self.pool.average()?;
        let Some(upper) = upper_index(lots, bound) else {
            return Ok(Vec::new());
        };
        lots[..=upper]
            .iter()
            .map(|lot| mul(&average, &lot.crypto_in))
            .collect()
    }

    fn extend_bound(&mut self, _: &[LotView], _: &LotState, _: usize) -> EngineResult<()> {
        Ok(())
    }
}

/// Austria's method: Altvermögen lots FIFO at their own basis, Neuvermögen
/// lots through per-pool moving averages, and Neu swaps at a zero-gain
/// override with the pool average carried to the incoming leg.
#[derive(Debug)]
pub(crate) struct MovingAverageAt {
    /// Each lot's regime, or the error classifying it raises. RP2 classifies
    /// lazily, so an error only surfaces where RP2 consults the lot.
    regimes: Vec<EngineResult<Regime>>,
    /// Each lot's `at_pool`, or the error parsing it raises.
    lot_pools: Vec<EngineResult<String>>,
    pools: BTreeMap<String, Pool>,
    /// Lots before this index are exhausted, so every scan skips them
    /// (an exhausted lot was classified when it was selected).
    exhausted_prefix: usize,
}

impl MovingAverageAt {
    pub fn new(entries: &[Entry], lots: &[LotView]) -> Self {
        MovingAverageAt {
            regimes: lots
                .iter()
                .map(|lot| lot_regime(&entries[lot.entry]))
                .collect(),
            lot_pools: lots
                .iter()
                .map(|lot| pool_id(&entries[lot.entry].notes))
                .collect(),
            pools: BTreeMap::new(),
            exhausted_prefix: 0,
        }
    }

    fn regime(&self, lot: usize) -> EngineResult<Regime> {
        self.regimes[lot].clone()
    }

    fn lot_pool(&self, lot: usize) -> EngineResult<&str> {
        self.lot_pools[lot].as_deref().map_err(Clone::clone)
    }

    /// The first lot of `regime` (and of `pool`, when given) with an
    /// available amount, scanning lots `0..=bound` in order.
    fn find(
        &mut self,
        lots: &[LotView],
        state: &LotState,
        bound: usize,
        regime: Regime,
        pool: Option<&str>,
    ) -> EngineResult<Option<(usize, Decimal)>> {
        let Some(upper) = upper_index(lots, bound) else {
            return Ok(None);
        };
        while self.exhausted_prefix <= upper {
            match &state.partial[self.exhausted_prefix] {
                Some(remaining) if !gt13(remaining, &zero())? => self.exhausted_prefix += 1,
                _ => break,
            }
        }
        for lot in self.exhausted_prefix..=upper {
            if self.regime(lot)? != regime {
                continue;
            }
            if let Some(pool) = pool {
                if self.lot_pool(lot)? != pool {
                    continue;
                }
            }
            match &state.partial[lot] {
                Some(remaining) if !gt13(remaining, &zero())? => continue,
                Some(remaining) => return Ok(Some((lot, remaining.clone()))),
                None => return Ok(Some((lot, lots[lot].crypto_in.clone()))),
            }
        }
        Ok(None)
    }

    /// Syncs the Neu lots of `pool` up to `bound` into that pool.
    fn sync_neu(
        &mut self,
        lots: &[LotView],
        state: &LotState,
        bound: usize,
        pool: &str,
    ) -> EngineResult<()> {
        let MovingAverageAt {
            regimes,
            lot_pools,
            pools,
            ..
        } = self;
        let target = pools.entry(pool.to_owned()).or_default();
        target.sync(lots, state, bound, |lot| {
            if regimes[lot].clone()? != Regime::Neu {
                return Ok(false);
            }
            Ok(lot_pools[lot].as_deref().map_err(Clone::clone)? == pool)
        })
    }

    fn pool_average(&self, pool: &str) -> EngineResult<Decimal> {
        self.pools
            .get(pool)
            .map_or_else(|| Ok(zero()), Pool::average)
    }

    fn seek_alt(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
    ) -> EngineResult<Option<Selection>> {
        match self.find(lots, state, bound, Regime::Alt, None)? {
            Some((lot, amount)) => state.take(lots, lot, amount).map(Some),
            None => Ok(None),
        }
    }

    fn seek_neu(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
        needed: &Decimal,
        event: &Entry,
        pool: &str,
    ) -> EngineResult<Option<Selection>> {
        self.sync_neu(lots, state, bound, pool)?;
        let Some((lot, amount)) = self.find(lots, state, bound, Regime::Neu, Some(pool))? else {
            self.reject_pool_mismatch(lots, state, bound, pool, event)?;
            return Ok(None);
        };
        let average = self.pool_average(pool)?;
        let taken = consumed(needed, &amount)?;
        self.pools
            .entry(pool.to_owned())
            .or_default()
            .deduct(&taken, &average)?;
        state.partial[lot] = Some(zero());
        if has_swap_link(&event.notes) {
            if swap_link_id(&event.notes)?.is_none() {
                return Err(EngineError::value(format!(
                    "Empty `at_swap_link=` marker on disposal. The id is required so RP2 can pair the incoming leg and carry the basis. Event: {}",
                    event.text
                )));
            }
            if event.transaction_type != TransactionType::Sell {
                return Err(EngineError::value(format!(
                    "`at_swap_link=` marker on non-SELL disposal (transaction_type={}). Crypto-to-crypto swap neutrality only applies to SELL-type disposals. Event: {}",
                    event.transaction_type.value().to_ascii_uppercase(),
                    event.text
                )));
            }
            // Zero gain at fee-aware proceeds; the average is carried.
            let unit = div(&event.fiat_taxable_amount(), event.crypto_balance_change())?;
            return Ok(Some(Selection {
                lot,
                amount,
                unit_cost_basis_override: Some(unit),
                taxable_event_unit_cost_basis: Some(average),
            }));
        }
        Ok(Some(Selection {
            lot,
            amount,
            unit_cost_basis_override: Some(average),
            taxable_event_unit_cost_basis: None,
        }))
    }

    /// RP2's diagnostic for a Neu disposal whose pool is empty while other
    /// Neu pools still hold lots.
    fn reject_pool_mismatch(
        &self,
        lots: &[LotView],
        state: &LotState,
        bound: usize,
        pool: &str,
        event: &Entry,
    ) -> EngineResult<()> {
        let Some(upper) = upper_index(lots, bound) else {
            return Ok(());
        };
        let mut others = BTreeSet::new();
        for lot in 0..=upper {
            if self.regime(lot)? != Regime::Neu {
                continue;
            }
            let lot_pool = self.lot_pool(lot)?;
            if lot_pool == pool {
                continue;
            }
            if let Some(remaining) = &state.partial[lot] {
                if !gt13(remaining, &zero())? {
                    continue;
                }
            }
            others.insert(lot_pool);
        }
        if others.is_empty() {
            return Ok(());
        }
        let others: Vec<&str> = others.into_iter().collect();
        Err(EngineError::value(format!(
            "Neuvermoegen disposal tagged at_pool={} has no available lots in that pool, but other Neu pool(s) {} still hold lots. This usually means the disposal was tagged with a different pool than its acquisition (e.g. funds moved between wallets/pools without re-tagging at_pool=). Event: {}",
            py_repr(pool),
            py_str_list(&others),
            event.text
        )))
    }
}

impl LotMethod for MovingAverageAt {
    fn seek(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
        needed: &Decimal,
        event: &Entry,
    ) -> EngineResult<Option<Selection>> {
        let pool = pool_id(&event.notes)?;
        match regime_from_notes(&event.notes)? {
            Some(Regime::Alt) => self.seek_alt(lots, state, bound),
            Some(Regime::Neu) => self.seek_neu(lots, state, bound, needed, event, &pool),
            None => {
                // Routed by availability at every seek; both regimes
                // available is ambiguous.
                let alt = self.find(lots, state, bound, Regime::Alt, None)?.is_some();
                let neu = self
                    .find(lots, state, bound, Regime::Neu, Some(&pool))?
                    .is_some();
                if alt && neu {
                    return Err(EngineError::value(format!(
                        "Ambiguous Austrian disposal: both Altvermoegen and Neuvermoegen lots are available (pool={pool}). Tag the disposal with `at_regime=alt` or `at_regime=neu` in notes. Event: {}",
                        event.text
                    )));
                }
                if alt {
                    self.seek_alt(lots, state, bound)
                } else {
                    self.seek_neu(lots, state, bound, needed, event, &pool)
                }
            }
        }
    }

    /// Neu lots report their pool's final average times `crypto_in`; Alt
    /// lots their effective basis.
    fn open_position_basis(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
    ) -> EngineResult<Vec<Decimal>> {
        let Some(upper) = upper_index(lots, bound) else {
            return Ok(Vec::new());
        };
        let mut neu_pools = BTreeSet::new();
        for lot in 0..=upper {
            if self.regime(lot)? == Regime::Neu {
                neu_pools.insert(self.lot_pool(lot)?.to_owned());
            }
        }
        for pool in &neu_pools {
            self.sync_neu(lots, state, bound, pool)?;
        }
        (0..=upper)
            .map(|lot| {
                if self.regime(lot)? == Regime::Neu {
                    mul(
                        &self.pool_average(self.lot_pool(lot)?)?,
                        &lots[lot].crypto_in,
                    )
                } else {
                    Ok(state.basis(lots, lot).clone())
                }
            })
            .collect()
    }

    fn extend_bound(&mut self, _: &[LotView], _: &LotState, _: usize) -> EngineResult<()> {
        Ok(())
    }
}
