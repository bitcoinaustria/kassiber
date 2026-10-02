//! The views RP2's `ComputedData` derives after the pass: gain/loss values,
//! yearly summaries, balances, sold percentages, and open positions.

use crate::cursor::Fragment;
use crate::decimal::Decimal;
use crate::entry::{Detail, Entry, TransactionType};
use crate::error::{EngineError, EngineResult};
use crate::methods::LotView;
use crate::model::{BalanceOut, Dec, EventRef, GainLossOut, OpenPositionOut, YearlyOut};
use crate::num::{add, div, equal_within, lt13, mul, sub, zero, BALANCE_EXPONENT};
use crate::time::elapsed_days;

/// A fragment's derived fiat values (RP2 computes them on access).
#[derive(Clone, Debug)]
pub(crate) struct FragmentValues {
    pub fiat_cost_basis: Decimal,
    pub proceeds: Decimal,
    pub fiat_gain: Decimal,
    pub long_term: bool,
}

/// `GainLoss`'s properties, each one 32-digit step in RP2's order:
/// proceeds `(fiat_taxable × a) ÷ balance_change`, basis
/// `(fiat_in_with_fee × a) ÷ crypto_in` (or `override × a`), and gain
/// `proceeds − basis`.
pub(crate) fn fragment_values(
    fragment: &Fragment,
    entries: &[Entry],
    lots: &[LotView],
    long_term_days: i64,
) -> EngineResult<FragmentValues> {
    let event = &entries[fragment.event];
    let amount = &fragment.crypto_amount;
    let proceeds = div(
        &mul(&event.fiat_taxable_amount(), amount)?,
        event.crypto_balance_change(),
    )?;
    let (fiat_cost_basis, long_term) = match fragment.lot {
        None => (zero(), false),
        Some(lot) => {
            let view = &lots[lot];
            let basis = match &fragment.unit_cost_basis_override {
                Some(unit) => mul(unit, amount)?,
                None => div(&mul(&view.fiat_in_with_fee, amount)?, &view.crypto_in)?,
            };
            let days = elapsed_days(&event.ts, &entries[view.entry].ts);
            (basis, days >= i128::from(long_term_days))
        }
    };
    let fiat_gain = sub(&proceeds, &fiat_cost_basis)?;
    Ok(FragmentValues {
        fiat_cost_basis,
        proceeds,
        fiat_gain,
        long_term,
    })
}

/// The boundary form of one fragment.
pub(crate) fn gain_loss_out(
    fragment: &Fragment,
    values: &FragmentValues,
    entries: &[Entry],
    lots: &[LotView],
) -> GainLossOut {
    let event = &entries[fragment.event];
    GainLossOut {
        event: EventRef {
            kind: event.kind,
            row: event.row,
        },
        lot: fragment.lot.map(|lot| lots[lot].row),
        crypto_amount: Dec(fragment.crypto_amount.clone()),
        fiat_cost_basis: Dec(values.fiat_cost_basis.clone()),
        proceeds: Dec(values.proceeds.clone()),
        fiat_gain: Dec(values.fiat_gain.clone()),
        unit_cost_basis_override: fragment.unit_cost_basis_override.clone().map(Dec),
        long_term: values.long_term,
        // Filled by the Austrian classification hook.
        at_category: None,
        at_category_error: None,
    }
}

struct YearlySums {
    year: i32,
    transaction_type: TransactionType,
    long_term: bool,
    crypto_amount: Decimal,
    fiat_amount: Decimal,
    fiat_cost_basis: Decimal,
    fiat_gain_loss: Decimal,
}

/// `yearly_gain_loss_list`: running sums per (event year in its own
/// offset, type, long-term) in fragment order, each starting from `0`, in
/// RP2's order (descending by `"<asset> <year> <LONG|SHORT> <type>"`).
pub(crate) fn yearly(
    asset: &str,
    fragments: &[Fragment],
    values: &[FragmentValues],
    entries: &[Entry],
) -> EngineResult<Vec<YearlyOut>> {
    let mut sums: Vec<YearlySums> = Vec::new();
    for (fragment, value) in fragments.iter().zip(values) {
        let event = &entries[fragment.event];
        let year = event.ts.year();
        let kind = event.transaction_type;
        let position = sums.iter().position(|s| {
            s.year == year && s.transaction_type == kind && s.long_term == value.long_term
        });
        let slot = match position {
            Some(position) => &mut sums[position],
            None => {
                sums.push(YearlySums {
                    year,
                    transaction_type: kind,
                    long_term: value.long_term,
                    crypto_amount: zero(),
                    fiat_amount: zero(),
                    fiat_cost_basis: zero(),
                    fiat_gain_loss: zero(),
                });
                let last = sums.len() - 1;
                &mut sums[last]
            }
        };
        slot.crypto_amount = add(&slot.crypto_amount, &fragment.crypto_amount)?;
        slot.fiat_amount = add(&slot.fiat_amount, &value.proceeds)?;
        slot.fiat_cost_basis = add(&slot.fiat_cost_basis, &value.fiat_cost_basis)?;
        slot.fiat_gain_loss = add(&slot.fiat_gain_loss, &value.fiat_gain)?;
    }
    let sort_key = |s: &YearlySums| {
        format!(
            "{asset} {} {} {}",
            s.year,
            if s.long_term { "LONG" } else { "SHORT" },
            s.transaction_type.value()
        )
    };
    sums.sort_by_cached_key(|s| std::cmp::Reverse(sort_key(s)));
    Ok(sums
        .into_iter()
        .map(|s| YearlyOut {
            year: s.year,
            transaction_type: s.transaction_type.value(),
            long_term: s.long_term,
            crypto_amount: Dec(s.crypto_amount),
            fiat_amount: Dec(s.fiat_amount),
            fiat_cost_basis: Dec(s.fiat_cost_basis),
            fiat_gain_loss: Dec(s.fiat_gain_loss),
        })
        .collect())
}

/// `get_open_position_in_lot_sold_percentage`: per lot, the running sum of
/// `amount ÷ crypto_in` over its fragments, starting from `0`.
pub(crate) fn sold_percentages(
    fragments: &[Fragment],
    lots: &[LotView],
) -> EngineResult<Vec<Decimal>> {
    let mut sold = vec![zero(); lots.len()];
    for fragment in fragments {
        if let Some(lot) = fragment.lot {
            let share = div(&fragment.crypto_amount, &lots[lot].crypto_in)?;
            sold[lot] = add(&sold[lot], &share)?;
        }
    }
    Ok(sold)
}

/// Open positions in lot-list order.
pub(crate) fn open_positions(
    lots: &[LotView],
    sold: &[Decimal],
    basis: &[Decimal],
) -> Vec<OpenPositionOut> {
    lots.iter()
        .enumerate()
        .map(|(index, lot)| OpenPositionOut {
            row: lot.row,
            sold_percentage: Dec(sold[index].clone()),
            fiat_in_with_fee: Dec(basis
                .get(index)
                .cloned()
                .unwrap_or_else(|| lot.fiat_in_with_fee.clone())),
        })
        .collect()
}

struct Account {
    exchange: String,
    holder: String,
    final_balance: Decimal,
    acquired: Decimal,
    sent: Decimal,
    received: Decimal,
}

/// The accounts in first-touch order.
#[derive(Default)]
struct Accounts {
    accounts: Vec<Account>,
}

impl Accounts {
    fn index(&mut self, exchange: &str, holder: &str) -> usize {
        if let Some(index) = self
            .accounts
            .iter()
            .position(|a| a.exchange == exchange && a.holder == holder)
        {
            return index;
        }
        self.accounts.push(Account {
            exchange: exchange.to_owned(),
            holder: holder.to_owned(),
            final_balance: zero(),
            acquired: zero(),
            sent: zero(),
            received: zero(),
        });
        self.accounts.len() - 1
    }

    /// RP2's check after a debit: a balance that is nonzero at 10 places and
    /// negative at 13 aborts.
    fn check(&self, index: usize, entry: &Entry) -> EngineResult<()> {
        let account = &self.accounts[index];
        let balance = &account.final_balance;
        if !equal_within(balance, &zero(), BALANCE_EXPONENT)? && lt13(balance, &zero())? {
            return Err(EngineError::value(format!(
                "{} balance of account \"{}\" (holder \"{}\") went negative ({balance}) on the following transaction: {}",
                entry.asset, account.exchange, account.holder, entry.text
            )));
        }
        Ok(())
    }
}

/// `balance_set`: replays IN, then INTRA, then OUT entries in a stable sort
/// by instant, and aborts when an account goes negative beyond the 10-place
/// tolerance. Accounts are ordered by `"<exchange>_<holder>"`.
pub(crate) fn balances(
    entries: &[Entry],
    ins: &[usize],
    outs: &[usize],
    intras: &[usize],
) -> EngineResult<Vec<BalanceOut>> {
    let mut order: Vec<usize> = ins.iter().chain(intras).chain(outs).copied().collect();
    order.sort_by_key(|&index| entries[index].ts.us);
    let mut accounts = Accounts::default();
    for index in order {
        let entry = &entries[index];
        match &entry.detail {
            Detail::In {
                exchange,
                holder,
                crypto_in,
                ..
            } => {
                let to = accounts.index(exchange, holder);
                let account = &mut accounts.accounts[to];
                account.acquired = add(&account.acquired, crypto_in)?;
                account.final_balance = add(&account.final_balance, crypto_in)?;
            }
            Detail::Intra {
                from_exchange,
                from_holder,
                to_exchange,
                to_holder,
                crypto_sent,
                crypto_received,
                ..
            } => {
                let from = accounts.index(from_exchange, from_holder);
                let to = accounts.index(to_exchange, to_holder);
                let sender = &mut accounts.accounts[from];
                sender.sent = add(&sender.sent, crypto_sent)?;
                let receiver = &mut accounts.accounts[to];
                receiver.received = add(&receiver.received, crypto_received)?;
                let sender = &mut accounts.accounts[from];
                sender.final_balance = sub(&sender.final_balance, crypto_sent)?;
                let receiver = &mut accounts.accounts[to];
                receiver.final_balance = add(&receiver.final_balance, crypto_received)?;
                accounts.check(from, entry)?;
            }
            Detail::Out {
                exchange,
                holder,
                crypto_out_no_fee,
                crypto_fee,
                ..
            } => {
                let from = accounts.index(exchange, holder);
                let account = &mut accounts.accounts[from];
                account.sent = add(&add(&account.sent, crypto_out_no_fee)?, crypto_fee)?;
                account.final_balance =
                    sub(&sub(&account.final_balance, crypto_out_no_fee)?, crypto_fee)?;
                accounts.check(from, entry)?;
            }
        }
    }
    let mut accounts = accounts.accounts;
    accounts.sort_by_cached_key(|a| format!("{}_{}", a.exchange, a.holder));
    Ok(accounts
        .into_iter()
        .map(|a| BalanceOut {
            exchange: a.exchange,
            holder: a.holder,
            final_balance: Dec(a.final_balance),
            acquired_balance: Dec(a.acquired),
            sent_balance: Dec(a.sent),
            received_balance: Dec(a.received),
        })
        .collect())
}
