//! Lot selection: which acquisition lot a disposal consumes next.
//!
//! A [`LotMethod`] answers one seek at a time, as RP2's accounting-method
//! plugins do. FIFO walks the lot list; LIFO, HIFO, and LOFO rank lots in a
//! [`PyHeap`] with RP2's sort keys and its non-transitive 13-place
//! comparison. Pool methods (moving averages, in `pool`) implement the same
//! trait and override [`LotMethod::open_position_basis`].

use std::cmp::Ordering;

use crate::decimal::Decimal;
use crate::entry::Entry;
use crate::error::{EngineError, EngineResult};
use crate::heap::PyHeap;
use crate::model::MethodName;
use crate::num::{cmp13, div, eq13, gt13, lt13, neg, zero};
use crate::pool::{MovingAverage, MovingAverageAt};

/// What a method needs to know about one lot (an `InTransaction`), in
/// lot-list order.
#[derive(Clone, Debug)]
pub(crate) struct LotView {
    /// Index of the lot's entry in the asset's entry list.
    pub entry: usize,
    pub row: i64,
    /// Absolute instant, microseconds.
    pub us: i64,
    /// `timestamp.timestamp()`, the heap keys' time component.
    pub posix: f64,
    pub crypto_in: Decimal,
    pub fiat_in_with_fee: Decimal,
}

/// Per-lot state shared by the cursor and the method: RP2's partial-amount
/// map and its basis-override map.
#[derive(Clone, Debug)]
pub(crate) struct LotState {
    /// `None` when the lot has no partial entry (untouched: `crypto_in` is
    /// available). A seek clears the selected lot's entry to zero; the cursor
    /// writes the leftover back before the next disposal.
    pub partial: Vec<Option<Decimal>>,
    /// The effective acquisition basis when it differs from
    /// `fiat_in_with_fee` (the Austrian runner's carried basis).
    pub basis_override: Vec<Option<Decimal>>,
}

impl LotState {
    pub fn new(lot_count: usize) -> Self {
        LotState {
            partial: vec![None; lot_count],
            basis_override: vec![None; lot_count],
        }
    }

    /// `get_fiat_in_with_fee`: the override if present, else the lot's own.
    pub fn basis<'a>(&'a self, lots: &'a [LotView], lot: usize) -> &'a Decimal {
        self.basis_override[lot]
            .as_ref()
            .unwrap_or(&lots[lot].fiat_in_with_fee)
    }

    /// The amount a seek may take from `lot`, or `None` when its partial
    /// entry is zero at 13 places (exhausted).
    fn available(&self, lots: &[LotView], lot: usize) -> EngineResult<Option<Decimal>> {
        match &self.partial[lot] {
            None => Ok(Some(lots[lot].crypto_in.clone())),
            Some(amount) if gt13(amount, &zero())? => Ok(Some(amount.clone())),
            Some(_) => Ok(None),
        }
    }

    /// The tail every lot-method seek shares once a lot is chosen: clear its
    /// partial entry and report the per-unit override when its effective
    /// basis differs from `fiat_in_with_fee`.
    pub fn take(
        &mut self,
        lots: &[LotView],
        lot: usize,
        amount: Decimal,
    ) -> EngineResult<Selection> {
        self.partial[lot] = Some(zero());
        let basis = self.basis(lots, lot);
        let unit_cost_basis_override = if eq13(basis, &lots[lot].fiat_in_with_fee)? {
            None
        } else {
            Some(div(basis, &lots[lot].crypto_in)?)
        };
        Ok(Selection {
            lot,
            amount,
            unit_cost_basis_override,
            taxable_event_unit_cost_basis: None,
        })
    }
}

/// The lot a seek selected.
#[derive(Clone, Debug)]
pub(crate) struct Selection {
    /// Position in the lot list.
    pub lot: usize,
    /// The lot's available amount.
    pub amount: Decimal,
    /// Per-unit basis replacing the lot's own (`GainLoss` basis becomes
    /// `override × amount`).
    pub unit_cost_basis_override: Option<Decimal>,
    /// The disposal's carried per-unit basis, for swap carries (pool methods
    /// under the Austrian runner).
    pub taxable_event_unit_cost_basis: Option<Decimal>,
}

/// One accounting method's lot selection for one asset run.
pub(crate) trait LotMethod {
    /// Selects the lot for the next fragment of `event`, considering lots
    /// `0..=bound` (every lot at or before the event's instant). `needed` is
    /// the event's remaining amount. `None` means no lot is available.
    fn seek(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
        needed: &Decimal,
        event: &Entry,
    ) -> EngineResult<Option<Selection>>;

    /// Full-quantity basis per lot for lots `0..=bound` after the replay
    /// (RP2's `get_open_position_basis`). Lot methods report each lot's
    /// effective basis; pool methods report the pool average times
    /// `crypto_in`.
    fn open_position_basis(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
    ) -> EngineResult<Vec<Decimal>> {
        self.extend_bound(lots, state, bound)?;
        Ok((0..=bound.min(lots.len().saturating_sub(1)))
            .map(|lot| state.basis(lots, lot).clone())
            .collect())
    }

    /// Advances the eligibility bound without seeking (RP2's
    /// `set_to_index`).
    fn extend_bound(
        &mut self,
        lots: &[LotView],
        state: &LotState,
        bound: usize,
    ) -> EngineResult<()>;
}

/// Builds the selection for `method` over one asset's lots.
pub(crate) fn lot_method<'a>(
    method: MethodName,
    entries: &'a [Entry],
    lots: &'a [LotView],
) -> Box<dyn LotMethod + 'a> {
    match method {
        MethodName::Fifo => Box::new(Fifo::default()),
        MethodName::Lifo => Box::new(Ranked::new(Rank::Lifo)),
        MethodName::Hifo => Box::new(Ranked::new(Rank::Hifo)),
        MethodName::Lofo => Box::new(Ranked::new(Rank::Lofo)),
        MethodName::MovingAverage => Box::new(MovingAverage::default()),
        MethodName::MovingAverageAt => Box::new(MovingAverageAt::new(entries, lots)),
    }
}

/// FIFO: the first lot in list order that is not exhausted.
#[derive(Debug, Default)]
pub(crate) struct Fifo {
    /// RP2's `from_index`: lots before it are known to be exhausted.
    from_index: usize,
}

impl LotMethod for Fifo {
    fn seek(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
        _needed: &Decimal,
        _event: &Entry,
    ) -> EngineResult<Option<Selection>> {
        let start = self.from_index;
        let mut selected = None;
        for lot in start..=bound.min(lots.len().saturating_sub(1)) {
            match state.available(lots, lot)? {
                Some(amount) => {
                    selected = Some((lot, amount));
                    break;
                }
                // An exhausted lot moves the start forward, as RP2's does.
                None => self.from_index += 1,
            }
        }
        match selected {
            Some((lot, amount)) if gt13(&amount, &zero())? => {
                Ok(Some(state.take(lots, lot, amount)?))
            }
            _ => Ok(None),
        }
    }

    fn extend_bound(&mut self, _: &[LotView], _: &LotState, _: usize) -> EngineResult<()> {
        Ok(())
    }
}

/// The heap-ranked methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rank {
    /// Latest instant first, then highest row.
    Lifo,
    /// Highest unit cost first, then earliest instant, then lowest row.
    Hifo,
    /// Lowest unit cost first, then earliest instant, then lowest row.
    Lofo,
}

/// RP2's `AcquiredLotSortKey` plus the lot it ranks.
#[derive(Clone, Debug)]
struct HeapItem {
    cost: Decimal,
    time: f64,
    row: i128,
    lot: usize,
}

/// Python's `(key, lot) < (key, lot)` for these items: the first key
/// component that is not equal decides, the cost by RP2's 13-place rule.
/// Rows are unique within an asset, so the lot never decides.
fn item_less(a: &HeapItem, b: &HeapItem) -> EngineResult<bool> {
    match cmp13(&a.cost, &b.cost)? {
        Ordering::Less => return Ok(true),
        Ordering::Greater => return Ok(false),
        Ordering::Equal => {}
    }
    if a.time != b.time {
        return Ok(a.time < b.time);
    }
    Ok(a.row < b.row)
}

/// LIFO, HIFO, and LOFO: lots ranked in a CPython-compatible heap.
#[derive(Debug)]
struct Ranked {
    rank: Rank,
    heap: PyHeap<HeapItem>,
    /// Lots `0..pushed` are in the heap (RP2's `to_index + 1`).
    pushed: usize,
}

impl Ranked {
    fn new(rank: Rank) -> Self {
        Ranked {
            rank,
            heap: PyHeap::new(),
            pushed: 0,
        }
    }

    /// RP2's sort key for `lot` given its effective basis.
    fn item(&self, lots: &[LotView], state: &LotState, lot: usize) -> EngineResult<HeapItem> {
        let view = &lots[lot];
        let basis = state.basis(lots, lot);
        if lt13(basis, &zero())? {
            return Err(EngineError::value(format!(
                "Parameter 'fiat_in_with_fee' has non-positive value {basis}"
            )));
        }
        let row = i128::from(view.row);
        let item = match self.rank {
            Rank::Lifo => HeapItem {
                cost: zero(),
                time: -view.posix,
                row: -row,
                lot,
            },
            Rank::Hifo | Rank::Lofo => {
                // RP2 ranks on the lot's own unit cost, then replaces it with
                // the effective one; both divisions run.
                let own = div(&view.fiat_in_with_fee, &view.crypto_in)?;
                let effective = div(basis, &view.crypto_in)?;
                let cost = if self.rank == Rank::Hifo {
                    neg(&own)?;
                    neg(&effective)?
                } else {
                    effective
                };
                HeapItem {
                    cost,
                    time: view.posix,
                    row,
                    lot,
                }
            }
        };
        Ok(item)
    }

    fn push(&mut self, lots: &[LotView], state: &LotState, lot: usize) -> EngineResult<()> {
        let item = self.item(lots, state, lot)?;
        self.heap.push(item, &mut item_less)
    }
}

impl LotMethod for Ranked {
    fn seek(
        &mut self,
        lots: &[LotView],
        state: &mut LotState,
        bound: usize,
        _needed: &Decimal,
        _event: &Entry,
    ) -> EngineResult<Option<Selection>> {
        self.extend_bound(lots, state, bound)?;
        let mut selected = None;
        while let Some(item) = self.heap.pop(&mut item_less)? {
            if let Some(amount) = state.available(lots, item.lot)? {
                selected = Some((item.lot, amount));
                break;
            }
        }
        match selected {
            Some((lot, amount)) if gt13(&amount, &zero())? => {
                // The partial entry is cleared before the lot is re-ranked.
                state.partial[lot] = Some(zero());
                self.push(lots, state, lot)?;
                Ok(Some(state.take(lots, lot, amount)?))
            }
            _ => Ok(None),
        }
    }

    fn extend_bound(
        &mut self,
        lots: &[LotView],
        state: &LotState,
        bound: usize,
    ) -> EngineResult<()> {
        let end = (bound + 1).min(lots.len());
        while self.pushed < end {
            self.push(lots, state, self.pushed)?;
            self.pushed += 1;
        }
        Ok(())
    }
}
