//! Wallet-scoped lot selection: a disposal takes its lots only from the
//! wallet that disposes, and a move carries the lots it takes to its
//! destination with their basis and acquisition time.
//!
//! It is every country's per-wallet pool: the wallet-by-wallet,
//! account-by-account application Treas. Reg. § 1.1012-1(j) requires in
//! the United States from 2025 (Rev. Proc. 2024-28), and Germany's
//! walletbezogene order of use (BMF 6 March 2025 Rn. 62); a rules country
//! may require it (`pool_scope` in its rules). RP2's own per-wallet engine
//! is not used: its second
//! pass lets a wallet dispose of a lot its transfer analysis already moved
//! away (`docs/reference/tax-country-us.md#rp2-per-wallet`). Here one
//! chronological pass owns every wallet's holdings:
//!
//! - Entries are taken by instant; at one instant, acquisitions first, then
//!   moves in the walk's order, then disposals, so a same-instant move can
//!   fund a disposal at its destination.
//! - A move's fee leaves the source first, as a taxable disposal chosen by
//!   the method, as RP2's universal pass consumes it; the principal then
//!   leaves the source by the same method and arrives as portions of the
//!   same lots. A move to the same wallet only pays its fee.
//! - The method orders a wallet's portions by their lot: FIFO by the lot's
//!   acquisition, LIFO the reverse, HIFO and LOFO by the lot's unit cost,
//!   ties broken by acquisition and row as RP2 breaks them. A portion keeps
//!   its lot's acquisition time, so holding periods continue across moves.
//! - Fragments name the original lot and its basis per unit, so
//!   composition, open positions and sold percentages read them as the
//!   universal pass's; they come out in RP2's taxable-event order.
//!
//! The plain moving average has a per-wallet form: each wallet keeps its
//! own average cost, the method takes the wallet's FIFO portions (whose
//! acquisitions date the holding period) at that average, and a move
//! carries its source's average cost with the units it moves (the pass
//! Germany's `moving_average_fifo_period` uses). The other pool and
//! matching methods have no per-wallet form here and are refused. A wallet
//! that lacks the units a disposal or move needs fails as RP2's lot engine
//! does when lots run out; an unpriced move fee fails rather than vanish
//! with its basis.
//!
//! [`transition_fragments`] runs the same pass with one universal pool
//! until a change instant (the United States' 2025-01-01), then allocates
//! the pool's remaining lots to the wallets by a standing rule and goes on
//! per wallet; see its documentation.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::cursor::{check_fragment, Fragment, LOTS_EXHAUSTED};
use crate::decimal::Decimal;
use crate::entry::{Detail, Entry};
use crate::error::{EngineError, EngineResult};
use crate::methods::LotView;
use crate::model::{Dec, EntryKind, MethodName, WalletHoldingOut};
use crate::num::{add, cmp13, div, gt13, mul, sub, zero};

/// A wallet: RP2's account, an exchange label and a holder.
type Wallet<'e> = (&'e str, &'e str);

/// The one pool every wallet's units are in before a transition. No wallet
/// label holds a NUL, so it names no wallet.
const UNIVERSAL: Wallet<'static> = ("\u{0}universal", "");

/// Part of a lot one wallet holds.
#[derive(Clone, Debug)]
struct Portion {
    lot: usize,
    amount: Decimal,
}

/// The order a method takes a wallet's portions in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Order {
    Fifo,
    Lifo,
    Hifo,
    Lofo,
}

impl Order {
    fn of(method: MethodName) -> EngineResult<Self> {
        Ok(match method {
            MethodName::Fifo => Order::Fifo,
            MethodName::Lifo => Order::Lifo,
            MethodName::Hifo => Order::Hifo,
            MethodName::Lofo => Order::Lofo,
            MethodName::MovingAverage => Order::Fifo,
            MethodName::MovingAverageAt
            | MethodName::MovingAverageFifoPeriod
            | MethodName::UkShareMatching
            | MethodName::JpTotalAverage
            | MethodName::JpMovingAverage => {
                return Err(EngineError::value(format!(
                    "Accounting method '{}' has no wallet-scoped form: choose FIFO, LIFO, HIFO, LOFO or MOVING_AVERAGE, or the global pool scope",
                    method.as_str()
                )))
            }
        })
    }
}

/// What a taken portion costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Basis {
    /// Its lot's own cost per unit (the lot methods).
    Lot,
    /// The wallet's moving average cost per unit when it is taken:
    /// acquisitions raise the wallet's cost, disposals take the average out,
    /// and a move carries its source's average to its destination.
    WalletAverage,
}

/// The per-wallet holdings of one asset during the pass.
struct Holdings<'a> {
    lots: &'a [LotView],
    order: Order,
    /// Each lot's unit cost, for HIFO and LOFO.
    unit_cost: Vec<Decimal>,
    wallets: HashMap<Wallet<'a>, Vec<Portion>>,
    basis: Basis,
    /// Under the wallet average, each wallet's total cost.
    cost: HashMap<Wallet<'a>, Decimal>,
}

impl<'a> Holdings<'a> {
    /// Whether portion `a` goes before portion `b`.
    fn before(&self, a: usize, b: usize) -> EngineResult<bool> {
        let (x, y) = (&self.lots[a], &self.lots[b]);
        let by_time = (x.us, x.row).cmp(&(y.us, y.row));
        Ok(match self.order {
            // The lot list is in acquisition order already.
            Order::Fifo => a < b,
            Order::Lifo => by_time == Ordering::Greater,
            Order::Hifo | Order::Lofo => {
                let cost = cmp13(&self.unit_cost[a], &self.unit_cost[b])?;
                let cost = if self.order == Order::Hifo {
                    cost.reverse()
                } else {
                    cost
                };
                match cost {
                    Ordering::Equal => by_time == Ordering::Less,
                    other => other == Ordering::Less,
                }
            }
        })
    }

    /// Adds `amount` of `lot` to `wallet`, merging with a portion of the
    /// same lot.
    fn put(&mut self, wallet: Wallet<'a>, lot: usize, amount: Decimal) -> EngineResult<()> {
        let portions = self.wallets.entry(wallet).or_default();
        match portions.iter_mut().find(|portion| portion.lot == lot) {
            Some(portion) => portion.amount = add(&portion.amount, &amount)?,
            None => portions.push(Portion { lot, amount }),
        }
        Ok(())
    }

    /// Under the wallet average, the wallet's cost per unit (`None` under
    /// the lots' own basis, or for a wallet that holds nothing).
    fn average(&self, wallet: Wallet<'a>) -> EngineResult<Option<Decimal>> {
        if self.basis == Basis::Lot {
            return Ok(None);
        }
        let quantity = self.quantity(wallet)?;
        if !gt13(&quantity, &zero())? {
            return Ok(None);
        }
        let cost = self.cost.get(&wallet).cloned().unwrap_or_else(zero);
        Ok(Some(div(&cost, &quantity)?))
    }

    /// The units `wallet` holds.
    fn quantity(&self, wallet: Wallet<'a>) -> EngineResult<Decimal> {
        let mut quantity = zero();
        for portion in self.wallets.get(&wallet).map(Vec::as_slice).unwrap_or(&[]) {
            quantity = add(&quantity, &portion.amount)?;
        }
        Ok(quantity)
    }

    /// Moves the cost of `taken` at `unit` per unit out of `from`, into `to`
    /// when the units arrive there. A wallet left empty keeps no cost.
    fn move_cost(
        &mut self,
        from: Wallet<'a>,
        to: Option<Wallet<'a>>,
        unit: &Option<Decimal>,
        taken: &[(usize, Decimal)],
    ) -> EngineResult<()> {
        let Some(unit) = unit else {
            return Ok(());
        };
        let mut moved = zero();
        for (_, amount) in taken {
            moved = add(&moved, &mul(unit, amount)?)?;
        }
        let left = if gt13(&self.quantity(from)?, &zero())? {
            sub(self.cost.get(&from).unwrap_or(&zero()), &moved)?
        } else {
            zero()
        };
        self.cost.insert(from, left);
        if let Some(to) = to {
            let cost = self.cost.entry(to).or_insert_with(zero);
            *cost = add(cost, &moved)?;
        }
        Ok(())
    }

    /// Takes `needed` from `wallet` for `event`, portion by portion in the
    /// method's order, among lots acquired at or before the event. Returns
    /// each `(lot, amount)` taken.
    fn take(
        &mut self,
        wallet: Wallet<'a>,
        needed: &Decimal,
        event: &Entry,
    ) -> EngineResult<Vec<(usize, Decimal)>> {
        let mut needed = needed.clone();
        let mut taken = Vec::new();
        loop {
            let portions = self.wallets.get(&wallet).map(Vec::as_slice).unwrap_or(&[]);
            let mut best: Option<usize> = None;
            for (index, portion) in portions.iter().enumerate() {
                if self.lots[portion.lot].us > event.ts.us || !gt13(&portion.amount, &zero())? {
                    continue;
                }
                best = match best {
                    Some(current) if !self.before(portion.lot, portions[current].lot)? => {
                        Some(current)
                    }
                    _ => Some(index),
                };
            }
            let Some(index) = best else {
                return Err(EngineError::value(format!(
                    "{LOTS_EXHAUSTED} in wallet {}: {}",
                    wallet.0, event.text
                )));
            };
            let portions = self
                .wallets
                .get_mut(&wallet)
                .expect("the wallet holds the portion");
            let portion = &mut portions[index];
            match cmp13(&needed, &portion.amount)? {
                Ordering::Greater => {
                    needed = sub(&needed, &portion.amount)?;
                    taken.push((portion.lot, std::mem::replace(&mut portion.amount, zero())));
                }
                _ => {
                    portion.amount = sub(&portion.amount, &needed)?;
                    taken.push((portion.lot, needed));
                    return Ok(taken);
                }
            }
        }
    }
}

/// Records `taken` as fragments of the event at `index`, at `unit` per
/// unit when the wallet's average replaces the lots' own cost.
fn emit(
    by_event: &mut HashMap<usize, Vec<Fragment>>,
    entries: &[Entry],
    lots: &[LotView],
    index: usize,
    taken: Vec<(usize, Decimal)>,
    unit: &Option<Decimal>,
) -> EngineResult<()> {
    let event = &entries[index];
    let out = by_event.entry(index).or_default();
    for (lot, amount) in taken {
        check_fragment(event, Some(&lots[lot]), &amount, unit.as_ref())?;
        out.push(Fragment {
            event: index,
            lot: Some(lot),
            crypto_amount: amount,
            unit_cost_basis_override: unit.clone(),
            cost_override: None,
        });
    }
    Ok(())
}

/// The fragments of one asset under the wallet pool scope, in RP2's
/// taxable-event order, and each wallet's remaining units with their own
/// basis (by exchange and holder). `events` are the asset's taxable events
/// in that order; `ins` its visible lots, in lot order.
pub(crate) fn fragments(
    entries: &[Entry],
    lots: &[LotView],
    ins: &[usize],
    events: &[usize],
    method: MethodName,
) -> EngineResult<(Vec<Fragment>, Vec<WalletHoldingOut>)> {
    let basis = if method == MethodName::MovingAverage {
        Basis::WalletAverage
    } else {
        Basis::Lot
    };
    fragments_with(entries, lots, ins, events, Order::of(method)?, basis)
}

/// The fragments of one asset that is identified across every wallet (the
/// global pool) before `change_us` and per wallet from then on (a lot
/// method's), and each wallet's remaining units, as [`fragments`].
///
/// Before the change one pool holds every wallet's lots: a disposal takes
/// from it by the method, a move only pays its fee from it, and each
/// wallet's own unit count is kept beside it. At the change (immediately
/// before the first entry at or after `change_us`, or at the end of a book
/// that never reaches it) the pool's remaining lots are allocated by a
/// standing rule, Rev. Proc. 2024-28 section 5.02(2)(b)'s global
/// allocation: the lots in the method's order (FIFO the earliest
/// acquisitions first, LIFO the latest, HIFO the highest unit cost, LOFO
/// the lowest, ties by acquisition and row), each wallet holding units
/// filled in turn, wallets in `wallet_order` (the book's wallets by when
/// they were added, so renaming one does not move basis between them),
/// then any it does not name by label and holder, until it has exactly
/// the units it held. A wallet count below zero, or counts that do not add
/// up to the pool's units, fail instead of allocating a guess.
pub(crate) fn transition_fragments(
    entries: &[Entry],
    lots: &[LotView],
    ins: &[usize],
    events: &[usize],
    method: MethodName,
    change_us: i64,
    wallet_order: &[String],
) -> EngineResult<(Vec<Fragment>, Vec<WalletHoldingOut>)> {
    let order = match method {
        MethodName::Fifo | MethodName::Lifo | MethodName::Hifo | MethodName::Lofo => {
            Order::of(method)?
        }
        _ => {
            return Err(EngineError::value(format!(
                "Accounting method '{}' has no change from the global to the wallet pool scope: choose FIFO, LIFO, HIFO or LOFO",
                method.as_str()
            )))
        }
    };
    pass(
        entries,
        lots,
        ins,
        events,
        order,
        Basis::Lot,
        Some((change_us, wallet_order)),
    )
}

/// [`fragments`] in `order`, with each taken portion costing its `basis`.
pub(crate) fn fragments_with(
    entries: &[Entry],
    lots: &[LotView],
    ins: &[usize],
    events: &[usize],
    order: Order,
    basis: Basis,
) -> EngineResult<(Vec<Fragment>, Vec<WalletHoldingOut>)> {
    pass(entries, lots, ins, events, order, basis, None)
}

/// Allocates the universal pool's portions to the wallets that hold units
/// (`counts`): portions in the method's order, wallets in `wallet_order`
/// (labels) and then by label and holder, each filled up to its count.
fn allocate<'a>(
    holdings: &mut Holdings<'a>,
    counts: &mut HashMap<Wallet<'a>, Decimal>,
    wallet_order: &[String],
) -> EngineResult<()> {
    let mut portions = holdings.wallets.remove(&UNIVERSAL).unwrap_or_default();
    portions.retain(|portion| !portion.amount.is_zero());
    // Insertion sort with the method's comparison, which can fail.
    let mut ordered: Vec<Portion> = Vec::with_capacity(portions.len());
    for portion in portions {
        let mut at = ordered.len();
        for (index, other) in ordered.iter().enumerate() {
            if holdings.before(portion.lot, other.lot)? {
                at = index;
                break;
            }
        }
        ordered.insert(at, portion);
    }
    let mut wallets: Vec<(Wallet<'a>, Decimal)> = counts.drain().collect();
    let rank = |wallet: &Wallet<'_>| {
        wallet_order
            .iter()
            .position(|label| label == wallet.0)
            .unwrap_or(usize::MAX)
    };
    wallets.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then_with(|| a.0.cmp(&b.0)));
    let mut pool = zero();
    for portion in &ordered {
        pool = add(&pool, &portion.amount)?;
    }
    let mut held = zero();
    for (wallet, count) in &wallets {
        if cmp13(count, &zero())? == Ordering::Less {
            return Err(EngineError::value(format!(
                "Cannot allocate the global pool to wallets at the change to per-wallet identification: wallet {} would hold {count} units",
                wallet.0
            )));
        }
        held = add(&held, count)?;
    }
    if cmp13(&held, &pool)? != Ordering::Equal {
        return Err(EngineError::value(format!(
            "Cannot allocate the global pool to wallets at the change to per-wallet identification: the wallets hold {held} units and the pool {pool}"
        )));
    }
    let mut next = ordered.into_iter();
    let mut current: Option<Portion> = None;
    for (wallet, count) in wallets {
        let mut needed = count;
        while gt13(&needed, &zero())? {
            let mut portion = match current.take() {
                Some(portion) => portion,
                None => next.next().ok_or_else(|| {
                    EngineError::runtime("Internal error: the global pool ran out while allocating")
                })?,
            };
            if cmp13(&portion.amount, &needed)? == Ordering::Greater {
                portion.amount = sub(&portion.amount, &needed)?;
                holdings.put(wallet, portion.lot, needed.clone())?;
                current = Some(portion);
                needed = zero();
            } else {
                needed = sub(&needed, &portion.amount)?;
                holdings.put(wallet, portion.lot, portion.amount)?;
            }
        }
    }
    Ok(())
}

/// Adds `amount` (negative to remove) to a wallet's count before the
/// change.
fn count<'a>(
    counts: &mut HashMap<Wallet<'a>, Decimal>,
    wallet: Wallet<'a>,
    amount: &Decimal,
    remove: bool,
) -> EngineResult<()> {
    let held = counts.entry(wallet).or_insert_with(zero);
    *held = if remove {
        sub(held, amount)?
    } else {
        add(held, amount)?
    };
    Ok(())
}

/// The wallet pass; with `change` (its instant and the allocation's wallet
/// order), one universal pool before it.
fn pass(
    entries: &[Entry],
    lots: &[LotView],
    ins: &[usize],
    events: &[usize],
    order: Order,
    basis: Basis,
    change: Option<(i64, &[String])>,
) -> EngineResult<(Vec<Fragment>, Vec<WalletHoldingOut>)> {
    let (change_us, wallet_order) = match change {
        Some((us, wallet_order)) => (Some(us), wallet_order),
        None => (None, &[][..]),
    };
    let unit_cost = lots
        .iter()
        .map(|lot| div(&lot.fiat_in_with_fee, &lot.crypto_in))
        .collect::<EngineResult<Vec<_>>>()?;
    let mut holdings = Holdings {
        lots,
        order,
        unit_cost,
        wallets: HashMap::new(),
        basis,
        cost: HashMap::new(),
    };
    let lot_of: HashMap<usize, usize> = ins
        .iter()
        .enumerate()
        .map(|(lot, &entry)| (entry, lot))
        .collect();

    // Every visible entry by instant: acquisitions, moves, then disposals,
    // each kind in entry order.
    let rank = |kind: EntryKind| match kind {
        EntryKind::In => 0,
        EntryKind::Intra => 1,
        EntryKind::Out => 2,
    };
    let mut order_of_pass: Vec<usize> = (0..entries.len())
        .filter(|&index| entries[index].ts.is_visible())
        .collect();
    order_of_pass.sort_by_key(|&index| (entries[index].ts.us, rank(entries[index].kind), index));

    let mut by_event: HashMap<usize, Vec<Fragment>> = HashMap::new();
    // Before the change: each wallet's own unit count beside the pool.
    let mut counts: HashMap<Wallet, Decimal> = HashMap::new();
    let mut universal = change_us.is_some();
    for index in order_of_pass {
        let entry = &entries[index];
        if universal && change_us.is_some_and(|change| entry.ts.us >= change) {
            allocate(&mut holdings, &mut counts, wallet_order)?;
            universal = false;
        }
        // The pool a wallet's units are taken from and put in.
        fn pool_of(universal: bool, wallet: Wallet<'_>) -> Wallet<'_> {
            if universal {
                UNIVERSAL
            } else {
                wallet
            }
        }
        let pool = |wallet| pool_of(universal, wallet);
        match &entry.detail {
            Detail::In {
                exchange,
                holder,
                crypto_in,
                ..
            } => {
                let lot = lot_of[&index];
                let wallet = (exchange.as_str(), holder.as_str());
                if universal {
                    count(&mut counts, wallet, crypto_in, false)?;
                }
                holdings.put(pool(wallet), lot, crypto_in.clone())?;
                if basis == Basis::WalletAverage {
                    let cost = holdings.cost.entry(wallet).or_insert_with(zero);
                    *cost = add(cost, &lots[lot].fiat_in_with_fee)?;
                }
                if entry.is_earning() {
                    check_fragment(entry, None, crypto_in, None)?;
                    by_event.entry(index).or_default().push(Fragment {
                        event: index,
                        lot: None,
                        crypto_amount: crypto_in.clone(),
                        unit_cost_basis_override: None,
                        cost_override: None,
                    });
                }
            }
            Detail::Out {
                exchange, holder, ..
            } => {
                let wallet = (exchange.as_str(), holder.as_str());
                if universal {
                    count(&mut counts, wallet, entry.crypto_balance_change(), true)?;
                }
                let wallet = pool(wallet);
                let unit = holdings.average(wallet)?;
                let taken = holdings.take(wallet, entry.crypto_balance_change(), entry)?;
                holdings.move_cost(wallet, None, &unit, &taken)?;
                emit(&mut by_event, entries, lots, index, taken, &unit)?;
            }
            Detail::Intra {
                from_exchange,
                from_holder,
                to_exchange,
                to_holder,
                crypto_received,
                crypto_fee,
                ..
            } => {
                let from = (from_exchange.as_str(), from_holder.as_str());
                let to = (to_exchange.as_str(), to_holder.as_str());
                if universal {
                    count(&mut counts, from, crypto_fee, true)?;
                    if from != to {
                        count(&mut counts, from, crypto_received, true)?;
                        count(&mut counts, to, crypto_received, false)?;
                    }
                }
                let (from, to) = (pool(from), pool(to));
                if gt13(crypto_fee, &zero())? {
                    if !entry.is_taxable()? {
                        return Err(EngineError::value(format!(
                            "A move's fee needs a price under the wallet pool scope, or its basis would vanish: {}",
                            entry.text
                        )));
                    }
                    let unit = holdings.average(from)?;
                    let taken = holdings.take(from, crypto_fee, entry)?;
                    holdings.move_cost(from, None, &unit, &taken)?;
                    emit(&mut by_event, entries, lots, index, taken, &unit)?;
                }
                if from != to && gt13(crypto_received, &zero())? {
                    let unit = holdings.average(from)?;
                    let taken = holdings.take(from, crypto_received, entry)?;
                    for (lot, amount) in &taken {
                        holdings.put(to, *lot, amount.clone())?;
                    }
                    holdings.move_cost(from, Some(to), &unit, &taken)?;
                }
            }
        }
    }
    if universal {
        // A book that ends before the change: its holdings as the rule
        // allocates them.
        allocate(&mut holdings, &mut counts, wallet_order)?;
    }
    let mut out = Vec::new();
    for index in events {
        out.extend(by_event.remove(index).unwrap_or_default());
    }
    let average_cost = std::mem::take(&mut holdings.cost);
    let mut wallets: Vec<_> = holdings.wallets.into_iter().collect();
    wallets.sort_by(|a, b| a.0.cmp(&b.0));
    let mut remaining = Vec::new();
    for ((exchange, holder), portions) in wallets {
        let mut quantity = zero();
        let mut cost_basis = zero();
        for portion in portions.iter().filter(|portion| !portion.amount.is_zero()) {
            let lot = &lots[portion.lot];
            quantity = add(&quantity, &portion.amount)?;
            cost_basis = add(
                &cost_basis,
                &div(
                    &mul(&lot.fiat_in_with_fee, &portion.amount)?,
                    &lot.crypto_in,
                )?,
            )?;
        }
        if basis == Basis::WalletAverage {
            cost_basis = average_cost
                .get(&(exchange, holder))
                .cloned()
                .unwrap_or_else(zero);
        }
        if gt13(&quantity, &zero())? {
            remaining.push(WalletHoldingOut {
                exchange: exchange.to_owned(),
                holder: holder.to_owned(),
                quantity: Dec(quantity),
                cost_basis: Dec(cost_basis),
            });
        }
    }
    Ok((out, remaining))
}
