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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A deposit, withdrawal or reward of its one leg, stored as this kind.
    Movement(&'static str),
    /// A buy (the leg arrives) or sell (it leaves), priced by the event.
    Trade,
    /// One leg of a crypto-to-crypto swap, unpriced; both legs carry the
    /// same reference (`swap_ref`), so the import pairs them and the
    /// profile's country decides the tax treatment.
    Swap(PyStr),
    /// A fee paid in a third asset (BEST, BNB, KCS): its own outbound row of
    /// that asset, a fee-only spend ([`assets::fee_only`]).
    Fee,
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

/// The provider's own BTC balance, read in the same consented sync after its
/// history, or why it is unknown.
pub(super) struct Balance {
    /// Whole msat, or why the provider's figure cannot be read exactly.
    pub msat: Result<i64, &'static str>,
    /// UTC ISO-8601 time the balance refers to.
    pub as_of: String,
    /// Which balance it is (`spot_available_plus_reserved`); documented per
    /// exchange.
    pub scope: &'static str,
}
impl Balance {
    /// `reported_balance`: the provider's figure, when it is known.
    pub fn reported(&self) -> Option<Py> {
        let msat = *self.msat.as_ref().ok()?;
        Some(dict([
            ("asset", s("BTC")),
            ("amount_msat", Py::from(msat)),
            ("as_of", s(self.as_of.as_str())),
            ("scope", s(self.scope)),
        ]))
    }
}

impl Event {
    /// The event of an exchange's built record entry: its `kind` text as the
    /// event's kind (`buy`/`sell` a trade, `fee` a third-asset fee).
    pub fn from_entry(entry: Entry) -> Self {
        let kind = match entry.kind {
            "buy" | "sell" => Kind::Trade,
            "fee" => Kind::Fee,
            kind => Kind::Movement(kind),
        };
        let (amount, fee) = match kind {
            // The entry already holds the fee-only shape; the event holds
            // the quantity.
            Kind::Fee => (entry.fee, PriceNumber::Finite(Decimal::zero())),
            _ => (entry.amount, entry.fee),
        };
        Self {
            occurred_at: entry.occurred_at,
            fee,
            pricing: entry.pricing,
            description: entry.description,
            raw: entry.raw,
            ..Self::one(entry.external_id, kind, entry.inbound, entry.asset, amount)
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
    /// Which assets this source reads (the exchange's API, or one export).
    reads: Reads,
    scope: Scope,
    /// The wallet's label, for hints that name the scope command.
    wallet: String,
    records: Vec<Py>,
    tally: Tally,
    unread: BTreeMap<String, i64>,
    fiat_only: i64,
    other: i64,
    internal: i64,
}

impl Mapper {
    pub fn new(exchange: &'static Descriptor, scope: Scope, wallet: &str) -> Self {
        Self::reading(exchange, exchange.reads, scope, wallet)
    }

    /// A mapper for one of the exchange's sources that reads `reads`.
    pub fn reading(
        exchange: &'static Descriptor,
        reads: Reads,
        scope: Scope,
        wallet: &str,
    ) -> Self {
        Self {
            exchange,
            reads,
            scope,
            wallet: wallet.to_owned(),
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
        let classed: Vec<(&str, Class)> = assets
            .iter()
            .map(|code| (*code, assets::class(code)))
            .collect();
        self.admit_classed(&classed)
    }

    /// [`Self::admit`] with the provider's own class for each asset (a stock
    /// or metal is `Excluded`, counted and never kept).
    pub fn admit_classed(&mut self, assets: &[(&str, Class)]) -> bool {
        let mut classed: Vec<(&str, Class)> = assets
            .iter()
            .copied()
            .filter(|(code, _)| !code.is_empty())
            .collect();
        classed.sort_unstable_by(|a, b| a.0.cmp(b.0));
        classed.dedup_by(|a, b| a.0 == b.0);
        let (codes, classes): (Vec<&str>, Vec<Class>) = classed.into_iter().unzip();
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
        // Kept when the scope keeps one of its crypto assets. An excluded
        // asset (a stock) is counted; traded against a kept crypto asset,
        // that leg is a disposal the exchange's parser must refuse.
        let mut kept = false;
        for (code, class) in codes.iter().zip(&classes) {
            match class {
                Class::Crypto if self.reads == Reads::BitcoinOnly => {
                    *self.unread.entry((*code).to_owned()).or_default() += 1;
                }
                Class::Crypto => kept |= self.tally.admit(self.scope, code, *class),
                Class::Excluded => {
                    self.tally.admit(self.scope, code, *class);
                }
                _ => {}
            }
        }
        if !kept {
            self.other += 1;
        }
        kept
    }

    /// Whether a leg of an event already admitted is kept: Bitcoin always,
    /// another crypto asset as [`Self::admit`] keeps it; a leg left out (a
    /// third-asset fee in a `bitcoin` wallet) is counted.
    pub fn keeps(&mut self, code: &str, class: Class) -> bool {
        match class {
            Class::Crypto if self.reads == Reads::BitcoinOnly => {
                *self.unread.entry(code.to_owned()).or_default() += 1;
                false
            }
            class => self.tally.admit(self.scope, code, class),
        }
    }

    /// Names, in the envelope's `next_step`, a history the exchange's
    /// importer does not read.
    pub fn notice(&mut self, text: String) {
        self.tally.notice(text);
    }

    /// The wallet's asset scope.
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// The command that sets this wallet's scope to `crypto`.
    pub fn keep_command(&self) -> String {
        assets::keep_command(&self.wallet)
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
        let (kind, reference) = match kind {
            Kind::Movement(kind) => (kind, None),
            Kind::Trade if inbound => ("buy", None),
            Kind::Trade => ("sell", None),
            Kind::Swap(reference) => ("swap", Some(reference)),
            Kind::Fee if inbound => {
                return Err(AppError::new("internal_error", "An incoming fee row"))
            }
            Kind::Fee => ("fee", None),
        };
        let (amount, fee) = match kind {
            "fee" => assets::fee_only(leg.amount),
            _ => (leg.amount, fee),
        };
        let mut record = record(Entry {
            provider: self.exchange.provider,
            external_id,
            occurred_at,
            inbound,
            asset: leg.asset,
            amount,
            fee,
            kind,
            raw,
            description,
            pricing,
        })?;
        if let Some(reference) = reference {
            kassiber_envelope::set(&mut record, "swap_ref", Py::Str(reference));
        }
        self.records.push(record);
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
