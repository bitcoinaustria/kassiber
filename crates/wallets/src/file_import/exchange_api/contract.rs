//! The contract every exchange importer shares
//! (docs/reference/exchange-contract.md). An exchange
//! is a transport for its API and/or a parser for its file exports, each
//! reading the provider's rows into [`Event`]s, plus the table of what its
//! row types mean. Everything after that is done here, once: which events
//! the wallet's asset scope keeps, the tally of those it leaves out, and the
//! records the coordinator stores.
//!
//! The asset scope is a pure filter over the same events. An event that moves
//! Bitcoin is mapped the same way in every scope; an event that moves no
//! Bitcoin is left out and counted before its type is interpreted, so a type
//! Kassiber does not map in another coin never stops a Bitcoin wallet.
use super::*;
use assets::{Class, Scope, Tally};

pub(super) mod registry;
#[cfg(test)]
mod tests;
pub(super) use registry::{Descriptor, Reads};

/// One asset amount an event moves.
pub(super) struct Leg {
    /// The canonical code (`BTC`, `ETH`, `EUR`).
    pub asset: String,
    /// As the provider states it; the record stores its absolute value.
    pub amount: PriceNumber,
}

/// What an event is, from the exchange's own table of its row types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A deposit, withdrawal or reward of its one leg, stored as this kind.
    Movement(&'static str),
    /// A buy (the leg arrives) or sell (it leaves), priced by the event.
    Trade,
}

/// One provider transaction, from its API or its file export.
pub(super) struct Event {
    /// The record's identity: `<exchange>:<the provider's id>`, or the id
    /// an existing book already stores for it.
    pub external_id: PyStr,
    pub occurred_at: Option<PyStr>,
    pub kind: Kind,
    /// What left the account besides the fee.
    pub sent: Option<Leg>,
    /// What reached the account.
    pub received: Option<Leg>,
    /// The fee, in the moving leg's asset.
    pub fee: PriceNumber,
    /// The exact execution value the provider states for a trade.
    pub pricing: Option<Pricing>,
    /// `None`: "<provider> <kind>".
    pub description: Option<PyStr>,
    /// The provider's evidence, stored as the record's `raw_json`.
    pub raw: Py,
}

impl Event {
    /// An event whose one leg arrives (`inbound`) or leaves, without a fee,
    /// time, price, description or evidence; set those with struct update.
    pub fn one(
        external_id: PyStr,
        kind: Kind,
        inbound: bool,
        asset: String,
        amount: PriceNumber,
    ) -> Self {
        let leg = Some(Leg { asset, amount });
        let (sent, received) = if inbound { (None, leg) } else { (leg, None) };
        Self {
            external_id,
            occurred_at: None,
            kind,
            sent,
            received,
            fee: PriceNumber::Finite(Decimal::zero()),
            pricing: None,
            description: None,
            raw: Py::Null,
        }
    }
}

/// What one import or sync of an exchange produced.
pub(super) struct Mapped {
    pub records: Vec<Py>,
    /// The rows left out by asset, for the envelope's `assets`.
    pub tally: Tally,
    /// The exchange's own counts of left-out events, where it reports them.
    pub skipped: Option<Py>,
}

/// Turns one exchange's events into records under one wallet's asset scope.
/// A parser asks [`Mapper::admit`] with an event's assets before it reads
/// anything else of the event, then hands every admitted event to
/// [`Mapper::push`], in the order the records are stored.
pub(super) struct Mapper {
    exchange: &'static Descriptor,
    scope: Scope,
    records: Vec<Py>,
    tally: Tally,
    unread: BTreeMap<String, i64>,
    fiat_only: i64,
    other: i64,
    internal: i64,
}

impl Mapper {
    pub fn new(exchange: &'static Descriptor, scope: Scope) -> Self {
        Self {
            exchange,
            scope,
            records: vec![],
            tally: Tally::default(),
            unread: BTreeMap::new(),
            fiat_only: 0,
            other: 0,
            internal: 0,
        }
    }

    /// Whether an event moving `assets` (canonical codes; blank codes are
    /// ignored) is mapped. Bitcoin always is. An event without Bitcoin is
    /// kept only when the exchange reads every asset and the wallet's scope
    /// is `crypto`; otherwise it is counted once per asset (fiat is never a
    /// holding) and left out.
    pub fn admit(&mut self, assets: &[&str]) -> bool {
        let mut codes: Vec<&str> = assets.iter().copied().filter(|c| !c.is_empty()).collect();
        codes.sort_unstable();
        codes.dedup();
        let classes: Vec<Class> = codes.iter().map(|code| assets::class(code)).collect();
        if classes.contains(&Class::Bitcoin) {
            return true;
        }
        if codes.is_empty() {
            return false;
        }
        if classes.iter().all(|class| *class == Class::Fiat) {
            self.fiat_only += 1;
            return false;
        }
        let crypto = codes
            .iter()
            .zip(&classes)
            .filter(|(_, class)| **class == Class::Crypto)
            .map(|(code, _)| *code);
        if self.exchange.reads == Reads::BitcoinOnly {
            for code in crypto {
                *self.unread.entry(code.to_owned()).or_default() += 1;
            }
            self.other += 1;
            return false;
        }
        let mut kept = true;
        for code in crypto {
            kept &= self.tally.admit(self.scope, code, Class::Crypto);
        }
        if !kept {
            self.other += 1;
        }
        kept
    }

    /// Counts an event left out whose assets the exchange does not name:
    /// fiat only, or another asset.
    pub fn leave(&mut self, fiat: bool) {
        match fiat {
            true => self.fiat_only += 1,
            false => self.other += 1,
        }
    }

    /// Maps one admitted event to its record.
    pub fn push(&mut self, event: Event) -> Result<(), AppError> {
        let Event {
            external_id,
            occurred_at,
            kind,
            sent,
            received,
            fee,
            pricing,
            description,
            raw,
        } = event;
        let (inbound, leg) = match (received, sent) {
            (Some(leg), None) => (true, leg),
            (None, Some(leg)) => (false, leg),
            _ => {
                return Err(AppError::new(
                    "internal_error",
                    "An exchange event without exactly one moving leg",
                ))
            }
        };
        let kind = match kind {
            Kind::Movement(kind) => kind,
            Kind::Trade if inbound => "buy",
            Kind::Trade => "sell",
        };
        self.records.push(record(Entry {
            provider: self.exchange.provider,
            external_id,
            occurred_at,
            inbound,
            asset: leg.asset,
            amount: leg.amount,
            fee,
            kind,
            raw,
            description,
            pricing,
        })?);
        Ok(())
    }

    /// Counts a move between the user's own balances on the exchange: the
    /// account's holdings do not change, so nothing is stored.
    pub fn internal(&mut self) {
        self.internal += 1;
    }

    pub fn finish(mut self) -> Mapped {
        self.tally.unread(self.exchange.provider, &self.unread);
        let skipped = self.exchange.skipped.map(|[fiat_only, other, internal]| {
            dict([
                (fiat_only, Py::from(self.fiat_only)),
                (other, Py::from(self.other)),
                (internal, Py::from(self.internal)),
            ])
        });
        Mapped {
            records: self.records,
            tally: self.tally,
            skipped,
        }
    }
}
