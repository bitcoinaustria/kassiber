//! The taxable-event pass for one asset (RP2's `TaxEngineCursor`).
//!
//! Events are consumed one at a time so a multi-asset runner can interleave
//! several cursors. An earn receipt emits one zero-basis fragment without
//! touching the lots. A disposal consumes its balance change from the lots
//! the method selects, splitting across lots and leaving a remainder on the
//! last one, which is written back before the next disposal.

use std::cmp::Ordering;

use crate::decimal::Decimal;
use crate::entry::Entry;
use crate::error::{EngineError, EngineResult};
use crate::methods::{LotMethod, LotState, LotView};
use crate::num::{cmp13, eq13, gt13, lt13, sub, zero};

/// RP2's message when no eligible lot is left for a disposal.
pub(crate) const LOTS_EXHAUSTED: &str =
    "Total in-transaction crypto value < total taxable crypto value";

/// One `GainLoss` as the pass creates it. The fiat fields are derived later
/// (see `outputs`), as RP2 computes them on access.
#[derive(Clone, Debug)]
pub(crate) struct Fragment {
    /// Index of the taxable event in the asset's entry list.
    pub event: usize,
    /// Position of the acquired lot in the lot list; `None` for earn
    /// receipts.
    pub lot: Option<usize>,
    pub crypto_amount: Decimal,
    pub unit_cost_basis_override: Option<Decimal>,
}

/// What consuming one taxable event produced.
#[derive(Clone, Debug)]
pub(crate) struct EventComputation {
    /// Index of the event in the asset's entry list.
    #[allow(dead_code)] // Callers know the event they consumed.
    pub event: usize,
    /// Number of fragments appended for it.
    pub fragments: usize,
    /// The carried per-unit basis a pool method reported (last fragment
    /// wins).
    pub taxable_event_unit_cost_basis: Option<Decimal>,
}

/// The per-asset cursor.
pub(crate) struct Cursor<'a> {
    entries: &'a [Entry],
    lots: &'a [LotView],
    /// Taxable events (entry indices) in processing order.
    events: &'a [usize],
    next: usize,
    state: LotState,
    /// The last lot touched and its leftover amount.
    current: Option<(usize, Decimal)>,
    method: Box<dyn LotMethod + 'a>,
}

impl<'a> Cursor<'a> {
    pub fn new(
        entries: &'a [Entry],
        lots: &'a [LotView],
        events: &'a [usize],
        method: Box<dyn LotMethod + 'a>,
    ) -> Self {
        Cursor {
            entries,
            lots,
            events,
            next: 0,
            state: LotState::new(lots.len()),
            current: None,
            method,
        }
    }

    /// The next taxable event, if any (interleaving runners order cursors
    /// by it).
    pub fn peek(&self) -> Option<&'a Entry> {
        self.events
            .get(self.next)
            .map(|&index| &self.entries[index])
    }

    /// Each lot's effective acquisition basis (RP2's
    /// `get_in_transaction_fiat_in_with_fee`).
    pub fn effective_basis(&self) -> Vec<Decimal> {
        (0..self.lots.len())
            .map(|lot| self.state.basis(self.lots, lot).clone())
            .collect()
    }

    /// Lot and basis state, for runners that set basis overrides (the
    /// Austrian runner writes carried basis here).
    pub fn state_mut(&mut self) -> &mut LotState {
        &mut self.state
    }

    /// Index of the last lot at or before `us`.
    fn bound(&self, us: i64) -> Option<usize> {
        self.lots.partition_point(|lot| lot.us <= us).checked_sub(1)
    }

    /// Consumes the next taxable event, appending its fragments to `out`.
    pub fn consume_next(
        &mut self,
        out: &mut Vec<Fragment>,
    ) -> EngineResult<Option<EventComputation>> {
        let Some(&index) = self.events.get(self.next) else {
            return Ok(None);
        };
        let event = &self.entries[index];
        let before = out.len();
        if event.is_earning() {
            let amount = event.crypto_balance_change().clone();
            check_fragment(event, None, &amount, None)?;
            out.push(Fragment {
                event: index,
                lot: None,
                crypto_amount: amount,
                unit_cost_basis_override: None,
            });
            self.next += 1;
            return Ok(Some(EventComputation {
                event: index,
                fragments: 1,
                taxable_event_unit_cost_basis: None,
            }));
        }

        if let Some((lot, amount)) = &self.current {
            check_non_negative("amount", amount)?;
            self.state.partial[*lot] = Some(amount.clone());
        }
        let mut carried: Option<Decimal> = None;
        let mut needed = sub(event.crypto_balance_change(), &zero())?;
        loop {
            let selection = match self.bound(event.ts.us) {
                Some(bound) => {
                    self.method
                        .seek(self.lots, &mut self.state, bound, &needed, event)?
                }
                None => None,
            };
            let Some(selection) = selection else {
                return Err(EngineError::value(LOTS_EXHAUSTED));
            };
            if let Some(basis) = &selection.taxable_event_unit_cost_basis {
                if let Some(previous) = &carried {
                    if !eq13(previous, basis)? {
                        return Err(EngineError::runtime(format!(
                            "Internal error: inconsistent taxable-event unit cost basis while processing {}: {previous} != {basis}",
                            event.text
                        )));
                    }
                }
                carried = Some(basis.clone());
            }
            check_non_negative("taxable_event_amount", &needed)?;
            check_non_negative("acquired_lot_amount", &selection.amount)?;
            let lot = selection.lot;
            let ordering = cmp13(&needed, &selection.amount)?;
            let amount = if ordering == Ordering::Greater {
                selection.amount.clone()
            } else {
                needed.clone()
            };
            check_fragment(
                event,
                Some(&self.lots[lot]),
                &amount,
                selection.unit_cost_basis_override.as_ref(),
            )?;
            out.push(Fragment {
                event: index,
                lot: Some(lot),
                crypto_amount: amount,
                unit_cost_basis_override: selection.unit_cost_basis_override,
            });
            match ordering {
                Ordering::Equal => {
                    self.current = Some((lot, zero()));
                    break;
                }
                Ordering::Less => {
                    self.current = Some((lot, sub(&selection.amount, &needed)?));
                    break;
                }
                Ordering::Greater => needed = sub(&needed, &selection.amount)?,
            }
        }
        self.next += 1;
        Ok(Some(EventComputation {
            event: index,
            fragments: out.len() - before,
            taxable_event_unit_cost_basis: carried,
        }))
    }

    /// Consumes every remaining event.
    pub fn run(&mut self, out: &mut Vec<Fragment>) -> EngineResult<()> {
        while self.consume_next(out)?.is_some() {}
        Ok(())
    }

    /// RP2's open-position snapshot: writes back the current leftover, then
    /// asks the method for each eligible lot's full-quantity basis. `None`
    /// when there is no lot.
    pub fn open_position_basis(&mut self) -> EngineResult<Option<Vec<Decimal>>> {
        if let Some((lot, amount)) = &self.current {
            check_non_negative("amount", amount)?;
            self.state.partial[*lot] = Some(amount.clone());
        }
        let Some(bound) = self.lots.len().checked_sub(1) else {
            return Ok(None);
        };
        self.method
            .open_position_basis(self.lots, &mut self.state, bound)
            .map(Some)
    }
}

/// `Configuration.type_check_positive_decimal(name, value)`.
fn check_non_negative(name: &str, value: &Decimal) -> EngineResult<()> {
    if lt13(value, &zero())? {
        return Err(EngineError::value(format!(
            "Parameter '{name}' has non-positive value {value}"
        )));
    }
    Ok(())
}

/// The checks RP2's `GainLoss` constructor runs. On the paths the cursor
/// takes they only fail by raising from a comparison.
fn check_fragment(
    event: &Entry,
    lot: Option<&LotView>,
    amount: &Decimal,
    unit_cost_basis_override: Option<&Decimal>,
) -> EngineResult<()> {
    check_non_negative("crypto_amount", amount)?;
    if eq13(amount, &zero())? {
        return Err(EngineError::value(
            "Parameter 'crypto_amount' has zero value",
        ));
    }
    let balance_change = event.crypto_balance_change();
    if event.is_earning() {
        if !eq13(amount, balance_change)? {
            return Err(EngineError::value(format!(
                "crypto_amount must be == taxable_event.crypto_balance_change for earn-typed taxable events, but they differ {amount} != {balance_change}"
            )));
        }
    } else if lot.is_none() {
        return Err(EngineError::type_error(
            "acquired_lot must not be None for non-earn-typed taxable_events",
        ));
    }
    if let Some(value) = unit_cost_basis_override {
        check_non_negative("unit_cost_basis_override", value)?;
    }
    // Python's `or` short-circuits, so the lot comparison may never run.
    let over = gt13(amount, balance_change)?
        || match lot {
            Some(lot) => gt13(amount, &lot.crypto_in)?,
            None => false,
        };
    if over {
        return Err(EngineError::value(format!(
            "crypto_amount ({amount}) is greater than taxable event amount ({balance_change}) or acquired-lot amount"
        )));
    }
    if let Some(lot) = lot {
        if event.ts.us < lot.us {
            return Err(EngineError::value(
                "Internal error: taxable event is earlier than its acquired lot",
            ));
        }
    }
    Ok(())
}
