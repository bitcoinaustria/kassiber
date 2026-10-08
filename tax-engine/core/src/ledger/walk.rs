//! The walk (design §4.6, §4.7): the adapter's `_prepare_rp2_asset_input`
//! (`tests/tax_oracle/adapter.py:736-1279`) with `_earliest_lot_contamination`
//! (`:708-733`), ported literally for the live paths.
//!
//! It visits one asset's normalized events and moves in walk order
//! ([`super::order`]), keeps the adapter's mirror of the lot engine's
//! balances (what each account holds, by wallet label, and one priced
//! amount for the asset), and either quarantines an item at a gate or
//! builds its entry for the lot engine:
//!
//! - a move must find its sent amount in its source account and in the
//!   priced amount. An ungrouped move that fails is quarantined on its out
//!   leg and, when its in leg is another transaction, on that leg too. A
//!   move of an atomic group is checked once for the whole group, on copies
//!   of the mirror, at the group's first move; if the group passes, each
//!   move is checked again at its own position, and a failure there
//!   quarantines that move and the group's later ones;
//! - an inbound event is quarantined when its kind looks like income the
//!   kind table does not list, or, on an Austrian book, when it is an
//!   airdrop or hard fork;
//! - an outbound event is quarantined when its kind is a disposition that
//!   is not a sale, when its account or the priced amount cannot cover it,
//!   or when it happens at or after the time the asset's lot state became
//!   uncertain. An Austrian swap-out the swap links mark skips the last
//!   three gates, so the lot engine sees the whole swap.
//!
//! An entry is built where the adapter calls its constructor, with the
//! adapter's arguments and the notes [`super::describe`] composes, and
//! checked by the lot engine's constructor ([`Entry::from_input`]) with the
//! book's names, so the first entry that fails fails as it did. Its text is
//! rendered once it is valid ([`super::text`]). The mirror compares exact
//! decimals; its arithmetic and the entries' values are CPython's.
//!
//! An asset with no inbound entry hands nothing to the lot engine: the
//! adapter drops its outbound and move entries (`:1261-1267`), though its
//! gate quarantines and transfer audit stay.
//!
//! The loan and channel roles the adapter checks are never set (it passes
//! no loan legs), and a move's transfer id is never set (B map §3.5), so
//! neither has a port.

use std::collections::{HashMap, HashSet};
use std::iter;

use serde_json::{json, Map, Value};

use crate::decimal::{Decimal, DecimalError};
use crate::entry::Entry;
use crate::error::{EngineError, EngineResult};
use crate::manifest::{
    inbound_transaction_type, kind_has_token, type_name, DetailKind, QuarantineReason,
    AT_ZERO_COST_KINDS, INBOUND_KIND_TYPES, INCOME_LIKE_KIND_TOKENS, NON_SALE_DISPOSAL_KIND_TOKENS,
};
use crate::model::{ConfigSpec, Dec, EntryInput, EntryKind};
use crate::num;

use super::describe::{self, EventNotes, TransferLeg, TransferNotes};
use super::normalize::{Event, Item, Normalized, Quarantine, Transfer};
use super::order::walk_order;
use super::request::{Country, Direction, IntraPair, Regime, Row};
use super::response::{AuditOut, Fiat};
use super::run::{AssetRows, Book};
use super::text::entry_text;

/// An entry the walk built: the lot engine's input, with its text, the
/// constructed entry (its stored and derived values, which composition
/// reads; it keeps no text), and the row whose time the entry has (an
/// event's row, a move's out leg).
#[derive(Clone, Debug)]
pub(crate) struct BuiltEntry {
    pub input: EntryInput,
    pub entry: Entry,
    pub row: usize,
}

/// One asset, walked (the adapter's `_RP2PreparedInput`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Walked {
    /// The inbound, then the outbound, then the move entries, each in walk
    /// order; `None` when the asset has no inbound entry.
    pub entries: Option<Vec<BuiltEntry>>,
    /// The gates' quarantines, in walk order. Normalize's are not repeated.
    pub quarantines: Vec<Quarantine>,
    /// One row per booked move, in walk order.
    pub audit: Vec<AuditOut>,
}

/// Walks one normalized asset. `config` holds the names the constructors
/// accept.
pub(crate) fn walk(
    book: &Book,
    part: &AssetRows,
    normalized: &Normalized,
    config: &ConfigSpec,
) -> EngineResult<Walked> {
    Walker::new(book, part, normalized, config).run(normalized)
}

/// Where an atomic group stands: approved by its check at its first move,
/// or failed, after which its remaining moves are skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupState {
    Approved,
    Failed,
}

/// What each account holds, by wallet label (RP2 keeps balances per
/// exchange and holder, and the holder is the book's). A missing label is
/// an account too, as Python's `None` key is.
#[derive(Clone, Debug, Default)]
struct Accounts<'n>(HashMap<Option<&'n str>, Decimal>);

impl<'n> Accounts<'n> {
    /// The account's amount; `Decimal("0")` until something moved.
    fn get(&self, label: Option<&'n str>) -> Decimal {
        self.0.get(&label).cloned().unwrap_or_else(num::zero)
    }

    fn add(&mut self, label: Option<&'n str>, amount: &Decimal) -> EngineResult<()> {
        let total = num::add(&self.get(label), amount)?;
        self.0.insert(label, total);
        Ok(())
    }

    fn sub(&mut self, label: Option<&'n str>, amount: &Decimal) -> EngineResult<()> {
        let total = num::sub(&self.get(label), amount)?;
        self.0.insert(label, total);
        Ok(())
    }
}

/// A float the adapter wrote into a detail, tagged for the reader.
fn float(value: &Decimal) -> Value {
    json!({ "float": value.to_canonical() })
}

fn label_value(label: Option<&str>) -> Value {
    label.map_or(Value::Null, Value::from)
}

/// A detail object from its fields; the reader serializes it with sorted
/// keys, so their order does not matter.
fn detail<const N: usize>(fields: [(&str, Value); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

/// The constructor's `notes`: `None` for empty notes, as the adapter's
/// entry builders write them.
fn notes(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// A finite stand-in for CPython's `Infinity`, which an entry cannot carry.
/// It is too large to round to 13 places at 32 digits, so the constructor's
/// first comparison of it fails with `InvalidOperation` where RP2's
/// comparison of `Infinity` does, after every check that reads the other
/// values first.
fn infinity_stand_in() -> EngineResult<Decimal> {
    Ok(Decimal::from_canonical("1E100")?)
}

/// A price or value as an entry argument: the value, or the stand-in for
/// `Infinity` and `true`.
fn argument(value: &Fiat) -> EngineResult<(Decimal, bool)> {
    Ok(match value {
        Fiat::Finite(value) => (value.clone(), false),
        Fiat::Infinity => (infinity_stand_in()?, true),
    })
}

/// An entry with the fields every kind sets.
fn entry_input(
    kind: EntryKind,
    row_index: i64,
    unique_id: &str,
    asset: &str,
    row: &Row,
) -> EntryInput {
    EntryInput {
        kind,
        row: row_index,
        unique_id: Some(unique_id.to_owned()),
        asset: asset.to_owned(),
        transaction_type: None,
        notes: None,
        exchange: None,
        holder: None,
        from_exchange: None,
        from_holder: None,
        to_exchange: None,
        to_holder: None,
        spot_price: None,
        crypto_in: None,
        crypto_fee: None,
        fiat_in_no_fee: None,
        fiat_in_with_fee: None,
        fiat_fee: None,
        crypto_out_no_fee: None,
        crypto_out_with_fee: None,
        fiat_out_no_fee: None,
        crypto_sent: None,
        crypto_received: None,
        ts: row.ts.input(),
        text: String::new(),
    }
}

/// `_earliest_lot_contamination`: normalize's contamination time, lowered
/// (as text) to the time of each event the gates drop without booking it:
/// an inbound kind that looks like unlisted income, on an Austrian book an
/// airdrop or hard fork, and an outbound disposition that is not a sale.
/// An event's time is compared whatever it is, empty or not.
fn lot_contamination(rows: &[Row], normalized: &Normalized, austrian: bool) -> Option<String> {
    let mut earliest = normalized.earliest_lot_contamination_at.clone();
    for event in &normalized.events {
        let row = &rows[event.row];
        let kind = row.kind.as_str();
        let contaminates = match row.direction {
            Direction::Inbound => {
                (!is_listed_kind(kind) && kind_has_token(kind, &INCOME_LIKE_KIND_TOKENS))
                    || (austrian && AT_ZERO_COST_KINDS.contains(&kind))
            }
            Direction::Outbound => kind_has_token(kind, &NON_SALE_DISPOSAL_KIND_TOKENS),
        };
        let time = row.ts.raw();
        if contaminates && earliest.as_deref().map_or(true, |since| time < since) {
            earliest = Some(time.to_owned());
        }
    }
    earliest
}

/// Whether the kind table lists `kind`.
fn is_listed_kind(kind: &str) -> bool {
    INBOUND_KIND_TYPES.iter().any(|(name, _)| *name == kind)
}

struct Walker<'w, 'a> {
    rows: &'a [Row],
    pairs: &'a [IntraPair],
    asset: &'a str,
    austrian: bool,
    /// The profile label, unstripped, as the adapter passes it.
    holder: Option<&'a str>,
    config: &'w ConfigSpec,
    accounts: Accounts<'w>,
    /// The asset's priced amount (`priced_available`).
    priced: Decimal,
    /// When the asset's lot state became uncertain (`first_drop_at`).
    first_drop_at: Option<String>,
    /// The next entry's row number (`row_index`).
    row_index: i64,
    ins: Vec<BuiltEntry>,
    outs: Vec<BuiltEntry>,
    moves: Vec<BuiltEntry>,
    quarantines: Vec<Quarantine>,
    audit: Vec<AuditOut>,
}

impl<'w, 'a> Walker<'w, 'a> {
    fn new(
        book: &Book<'a>,
        part: &AssetRows<'a>,
        normalized: &'w Normalized,
        config: &'w ConfigSpec,
    ) -> Self {
        let austrian = matches!(book.request.policy.country, Country::At {});
        Walker {
            rows: book.rows,
            pairs: &book.request.intra_pairs,
            asset: part.asset,
            austrian,
            holder: book.request.holder.as_deref(),
            config,
            accounts: Accounts::default(),
            priced: num::zero(),
            first_drop_at: lot_contamination(book.rows, normalized, austrian),
            row_index: 1,
            ins: Vec::new(),
            outs: Vec::new(),
            moves: Vec::new(),
            quarantines: Vec::new(),
            audit: Vec::new(),
        }
    }

    fn run(mut self, normalized: &'w Normalized) -> EngineResult<Walked> {
        let order = walk_order(self.rows, normalized);
        let events: HashMap<usize, &Event> = normalized
            .events
            .iter()
            .map(|event| (event.row, event))
            .collect();
        let transfers: HashMap<usize, &Transfer> = normalized
            .transfers
            .iter()
            .map(|transfer| (transfer.out, transfer))
            .collect();
        // Each group's moves, in walk order.
        let mut groups: HashMap<&str, Vec<&Transfer>> = HashMap::new();
        for item in &order {
            if let Item::Transfer(out) = item {
                let transfer = transfers[out];
                if let Some(group_id) = group_of(transfer) {
                    groups.entry(group_id).or_default().push(transfer);
                }
            }
        }
        let mut states: HashMap<&str, GroupState> = HashMap::new();
        for item in order {
            match item {
                Item::Transfer(out) => self.transfer(transfers[&out], &groups, &mut states)?,
                Item::Event(row) => {
                    let event = events[&row];
                    match self.rows[row].direction {
                        Direction::Inbound => self.inbound(event)?,
                        Direction::Outbound => self.outbound(event)?,
                    }
                }
            }
        }
        let entries = (!self.ins.is_empty()).then(|| {
            let mut entries = self.ins;
            entries.append(&mut self.outs);
            entries.append(&mut self.moves);
            entries
        });
        Ok(Walked {
            entries,
            quarantines: self.quarantines,
            audit: self.audit,
        })
    }

    fn quarantine(&mut self, row: usize, reason: QuarantineReason, detail: Map<String, Value>) {
        // Every gate's detail is its own template's.
        self.quarantines.push(Quarantine {
            row,
            reason,
            detail_kind: DetailKind::Plain,
            detail,
        });
    }

    fn anchor(&self, row: usize) -> &'a str {
        &self.rows[row].anchor
    }

    /// `_transfer_gate_failure` against an account state and priced amount.
    fn gate(
        &self,
        transfer: &'w Transfer,
        accounts: &Accounts<'w>,
        priced: &Decimal,
    ) -> Option<(QuarantineReason, Map<String, Value>)> {
        let from = transfer.from_wallet_label.as_deref();
        let available = accounts.get(from);
        if available < transfer.sent {
            return Some((
                QuarantineReason::InsufficientLots,
                detail([
                    ("from_wallet", label_value(from)),
                    ("asset", Value::from(self.asset)),
                    ("required", float(&transfer.sent)),
                    ("available", float(&available)),
                ]),
            ));
        }
        if *priced < transfer.sent {
            return Some((
                QuarantineReason::MissingCostBasis,
                detail([
                    ("from_wallet", label_value(from)),
                    ("asset", Value::from(self.asset)),
                    ("required", float(&transfer.sent)),
                    ("priced_available", float(priced)),
                ]),
            ));
        }
        None
    }

    /// A move at its position in the walk (`:996-1066`).
    fn transfer(
        &mut self,
        transfer: &'w Transfer,
        groups: &HashMap<&'w str, Vec<&'w Transfer>>,
        states: &mut HashMap<&'w str, GroupState>,
    ) -> EngineResult<()> {
        let Some(group_id) = group_of(transfer) else {
            if let Some((reason, detail)) = self.gate(transfer, &self.accounts, &self.priced) {
                self.ungrouped_quarantines(transfer, reason, detail);
                return Ok(());
            }
            return self.apply(transfer);
        };
        let group = &groups[group_id];
        match states.get(group_id) {
            Some(GroupState::Approved) => {
                // Checked again at its own position: a spend since the
                // group's check may have drained the source.
                if let Some((reason, detail)) = self.gate(transfer, &self.accounts, &self.priced) {
                    states.insert(group_id, GroupState::Failed);
                    let position = group
                        .iter()
                        .position(|leg| leg.out == transfer.out)
                        .ok_or_else(|| {
                            EngineError::runtime("Internal error: a move is missing from its group")
                        })?;
                    self.group_quarantines(&group[position..], transfer, reason, detail);
                    return Ok(());
                }
                self.apply(transfer)
            }
            Some(GroupState::Failed) => Ok(()),
            None => {
                states.insert(group_id, GroupState::Failed);
                let mut accounts = self.accounts.clone();
                let mut priced = self.priced.clone();
                for &candidate in group {
                    if let Some((reason, detail)) = self.gate(candidate, &accounts, &priced) {
                        self.group_quarantines(group, candidate, reason, detail);
                        return Ok(());
                    }
                    accounts.sub(candidate.from_wallet_label.as_deref(), &candidate.sent)?;
                    accounts.add(candidate.to_wallet_label.as_deref(), &candidate.received)?;
                    priced = num::sub(&priced, &candidate.fee)?;
                }
                states.insert(group_id, GroupState::Approved);
                self.apply(transfer)
            }
        }
    }

    /// `_append_transfer_gate_quarantine` (`:833-860`): the out leg, then
    /// the in leg unless it is the same transaction.
    fn ungrouped_quarantines(
        &mut self,
        transfer: &Transfer,
        reason: QuarantineReason,
        fields: Map<String, Value>,
    ) {
        self.quarantine(transfer.out, reason, fields.clone());
        let (out_anchor, in_anchor) = (self.anchor(transfer.out), self.anchor(transfer.inbound));
        if in_anchor == out_anchor {
            return;
        }
        let mut fields = fields;
        fields.insert(String::from("paired_leg"), Value::from("inbound"));
        fields.insert(
            String::from("blocked_by_transaction_id"),
            Value::from(out_anchor),
        );
        self.quarantine(transfer.inbound, reason, fields);
    }

    /// `_append_group_gate_quarantines` (`:862-903`): the failed move first,
    /// then the rest of `group`, each transaction named once.
    fn group_quarantines(
        &mut self,
        group: &[&Transfer],
        failed: &Transfer,
        reason: QuarantineReason,
        fields: Map<String, Value>,
    ) {
        let group_id = failed.group_id.clone().unwrap_or_default();
        let mut seen = HashSet::new();
        let rest = group.iter().copied().filter(|leg| leg.out != failed.out);
        for leg in iter::once(failed).chain(rest) {
            for (side_out, row) in [(true, leg.out), (false, leg.inbound)] {
                if !seen.insert(self.anchor(row)) {
                    continue;
                }
                if leg.out == failed.out && side_out {
                    let mut fields = fields.clone();
                    fields.insert(
                        String::from("transfer_group_id"),
                        Value::from(group_id.as_str()),
                    );
                    self.quarantine(row, reason, fields);
                } else {
                    let flag = detail([
                        ("asset", Value::from(self.asset)),
                        ("from_wallet", label_value(leg.from_wallet_label.as_deref())),
                        ("to_wallet", label_value(leg.to_wallet_label.as_deref())),
                        ("direction", Value::from("transfer")),
                        ("paired_leg", Value::from(!side_out)),
                        ("transfer_group_id", Value::from(group_id.as_str())),
                        ("blocked_by_reason", Value::from(reason.as_str())),
                    ]);
                    self.quarantine(row, QuarantineReason::DerivedTransferGroupBlocked, flag);
                }
            }
        }
    }

    /// `_apply_transfer_to_rp2` (`:905-986`): the move's entry, the mirror,
    /// and its audit row.
    fn apply(&mut self, transfer: &'w Transfer) -> EngineResult<()> {
        let rows = self.rows;
        let source = self.pairs[transfer.pair].source.as_deref();
        let regime = transfer.at_regime.map(Regime::as_str);
        let move_notes = describe::transfer_notes(
            &TransferNotes {
                regime,
                pool: Some(transfer.from_cost_basis_pool_id),
                pairing_source: source,
                description: &transfer.description,
            },
            self.austrian,
        );
        let (spot, infinite) = match &transfer.spot_price {
            Some(spot) => argument(spot)?,
            None => (num::zero(), false),
        };
        let out_row = &rows[transfer.out];
        let mut input = entry_input(
            EntryKind::Intra,
            self.row_index,
            &out_row.id,
            self.asset,
            out_row,
        );
        input.notes = notes(move_notes);
        input.from_exchange = transfer.from_wallet_label.clone();
        input.from_holder = self.holder.map(str::to_owned);
        input.to_exchange = transfer.to_wallet_label.clone();
        input.to_holder = self.holder.map(str::to_owned);
        input.spot_price = Some(Dec(spot.clone()));
        input.crypto_sent = Some(Dec(transfer.sent.clone()));
        input.crypto_received = Some(Dec(transfer.received.clone()));
        self.build(input, transfer.out, infinite)?;
        self.row_index += 1;
        let (from, to) = (
            transfer.from_wallet_label.as_deref(),
            transfer.to_wallet_label.as_deref(),
        );
        self.accounts.sub(from, &transfer.sent)?;
        self.accounts.add(to, &transfer.received)?;
        self.priced = num::sub(&self.priced, &transfer.fee)?;
        let in_row = &rows[transfer.inbound];
        let sent_msat = u128::from(out_row.amount_msat) + u128::from(out_row.fee_msat);
        let received_msat = u128::from(in_row.amount_msat);
        let fee_msat = sent_msat.checked_sub(received_msat).ok_or_else(|| {
            EngineError::runtime("Internal error: a move received more than it sent")
        })?;
        let at_pool = if self.austrian {
            describe::wire_pool(Some(transfer.from_cost_basis_pool_id))
        } else {
            None
        };
        let description = describe::transfer_leg_description(&TransferLeg {
            regime,
            flows: transfer.regime_flows.as_ref(),
            pool: at_pool.as_deref(),
            from_label: from,
            to_label: to,
            pairing_source: source,
        });
        self.audit.push(AuditOut {
            pair: transfer.pair,
            sent: Dec(transfer.sent.clone()),
            received: Dec(transfer.received.clone()),
            fee: Dec(transfer.fee.clone()),
            sent_msat,
            received_msat,
            fee_msat,
            spot_price: transfer.spot_price.as_ref().map(|_| Dec(spot)),
            at_regime: transfer.at_regime,
            regime_flows: transfer.regime_flows.clone(),
            at_pool,
            description,
        });
        Ok(())
    }

    /// The notes of an event's entry.
    fn event_notes(&self, event: &Event) -> Option<String> {
        notes(describe::event_notes(
            &EventNotes {
                regime: event.at_regime.map(Regime::as_str),
                regime_basis: event.at_regime_basis,
                pool: Some(event.cost_basis_pool_id),
                swap_link: event.at_swap_link.as_deref(),
                description: &event.description,
            },
            self.austrian,
        ))
    }

    /// An inbound event (`:1068-1129`).
    fn inbound(&mut self, event: &'w Event) -> EngineResult<()> {
        let row = &self.rows[event.row];
        let kind = row.kind.as_str();
        let wallet = event.wallet_label.as_deref();
        if self.austrian && AT_ZERO_COST_KINDS.contains(&kind) {
            let fields = detail([
                ("wallet", label_value(wallet)),
                ("asset", Value::from(self.asset)),
                ("kind", Value::from(kind)),
                ("tax_country", Value::from("at")),
                ("valuation_mode", Value::from("zero_cost")),
                (
                    "detail",
                    Value::from(
                        "Austrian airdrop/hardfork valuation requires unsupported zero-cost semantics; a label does not prove eligibility.",
                    ),
                ),
            ]);
            self.quarantine(
                event.row,
                QuarantineReason::AcquisitionValuationUnsupported,
                fields,
            );
            return Ok(());
        }
        if !is_listed_kind(kind) && kind_has_token(kind, &INCOME_LIKE_KIND_TOKENS) {
            let fields = detail([
                ("wallet", label_value(wallet)),
                ("asset", Value::from(self.asset)),
                ("direction", Value::from("inbound")),
                ("kind", Value::from(kind)),
            ]);
            self.quarantine(event.row, QuarantineReason::UnclassifiedIncomeKind, fields);
            return Ok(());
        }
        self.accounts.add(wallet, &event.amount)?;
        let (spot, spot_infinite) = argument(&event.spot_price)?;
        let (value, value_infinite) = argument(&event.fiat_value)?;
        let mut input = entry_input(EntryKind::In, self.row_index, &row.id, self.asset, row);
        input.transaction_type = Some(type_name(inbound_transaction_type(kind)));
        input.notes = self.event_notes(event);
        input.exchange = event.wallet_label.clone();
        input.holder = self.holder.map(str::to_owned);
        input.spot_price = Some(Dec(spot));
        input.crypto_in = Some(Dec(event.amount.clone()));
        input.fiat_in_no_fee = Some(Dec(value.clone()));
        input.fiat_in_with_fee = Some(Dec(value));
        input.fiat_fee = Some(Dec(num::zero()));
        self.build(input, event.row, spot_infinite || value_infinite)?;
        self.priced = num::add(&self.priced, &event.amount)?;
        self.row_index += 1;
        Ok(())
    }

    /// An outbound event (`:1131-1259`).
    fn outbound(&mut self, event: &'w Event) -> EngineResult<()> {
        let row = &self.rows[event.row];
        let needed = num::add(&event.amount, &event.fee)?;
        if !needed.is_positive() {
            return Ok(());
        }
        let kind = row.kind.as_str();
        let wallet = event.wallet_label.as_deref();
        if kind_has_token(kind, &NON_SALE_DISPOSAL_KIND_TOKENS) {
            // Availability stays: the lot engine keeps the lot, and the
            // event taints the lot state instead.
            let fields = detail([
                ("wallet", label_value(wallet)),
                ("asset", Value::from(self.asset)),
                ("direction", Value::from("outbound")),
                ("kind", Value::from(kind)),
            ]);
            self.quarantine(event.row, QuarantineReason::NonSaleDisposalKind, fields);
            return Ok(());
        }
        // A marked Austrian swap-out may depend on another asset's swap-in
        // at the same time; the lot engine sees the whole swap instead.
        let marked = event
            .at_swap_link
            .as_deref()
            .is_some_and(|link| !link.is_empty());
        let available = self.accounts.get(wallet);
        if available < needed && !marked {
            let fields = detail([
                ("wallet", label_value(wallet)),
                ("asset", Value::from(self.asset)),
                ("required", float(&needed)),
                ("available", float(&available)),
            ]);
            self.quarantine(event.row, QuarantineReason::InsufficientLots, fields);
            return Ok(());
        }
        if self.priced < needed && !marked {
            let fields = detail([
                ("wallet", label_value(wallet)),
                ("asset", Value::from(self.asset)),
                ("required", float(&needed)),
                ("priced_available", float(&self.priced)),
            ]);
            self.quarantine(event.row, QuarantineReason::MissingCostBasis, fields);
            return Ok(());
        }
        if let Some(since) = &self.first_drop_at {
            if !marked && row.ts.raw() >= since.as_str() {
                let fields = detail([
                    ("wallet", label_value(wallet)),
                    ("asset", Value::from(self.asset)),
                    ("required", float(&needed)),
                    ("lot_state_uncertain_since", Value::from(since.as_str())),
                ]);
                self.quarantine(
                    event.row,
                    QuarantineReason::BasisProvenanceIncomplete,
                    fields,
                );
                return Ok(());
            }
        }
        // The constructor's arguments, evaluated in Python's order: only
        // `fee * spot_price` can fail before the constructor runs.
        let sell = event.amount.is_positive();
        let (spot, spot_infinite) = argument(&event.spot_price)?;
        let (fiat_fee, fee_infinite) = match &event.spot_price {
            Fiat::Finite(price) => (num::mul(&event.fee, price)?, false),
            Fiat::Infinity if event.fee.is_zero() => {
                return Err(DecimalError::InvalidOperation.into())
            }
            Fiat::Infinity => (infinity_stand_in()?, true),
        };
        let mut input = entry_input(EntryKind::Out, self.row_index, &row.id, self.asset, row);
        input.transaction_type = Some(String::from(if sell { "SELL" } else { "FEE" }));
        input.notes = self.event_notes(event);
        input.exchange = event.wallet_label.clone();
        input.holder = self.holder.map(str::to_owned);
        input.spot_price = Some(Dec(spot));
        input.crypto_out_no_fee = Some(Dec(event.amount.clone()));
        input.crypto_fee = Some(Dec(event.fee.clone()));
        let mut value_infinite = false;
        if sell {
            let (value, infinite) = argument(&event.fiat_value)?;
            value_infinite = infinite;
            input.fiat_out_no_fee = Some(Dec(value));
        }
        input.fiat_fee = Some(Dec(fiat_fee));
        self.build(
            input,
            event.row,
            spot_infinite || fee_infinite || value_infinite,
        )?;
        self.accounts.sub(wallet, &needed)?;
        self.priced = num::sub(&self.priced, &needed)?;
        self.row_index += 1;
        Ok(())
    }

    /// Runs the constructor on `input` with the book's names and keeps the
    /// entry with its text. An argument that stands in for `Infinity` always
    /// fails the constructor; one that did not would be a defect.
    fn build(&mut self, mut input: EntryInput, row: usize, infinite: bool) -> EngineResult<()> {
        let entry = Entry::from_input(&input, Some(self.config))?;
        if infinite {
            return Err(EngineError::runtime(
                "Internal error: an entry was built from an infinite value",
            ));
        }
        input.text = entry_text(&entry)?;
        let kind = entry.kind;
        let built = BuiltEntry { input, entry, row };
        match kind {
            EntryKind::In => self.ins.push(built),
            EntryKind::Out => self.outs.push(built),
            EntryKind::Intra => self.moves.push(built),
        }
        Ok(())
    }
}

/// A move's atomic group, when it has one (`if transfer.group_id:`).
fn group_of(transfer: &Transfer) -> Option<&str> {
    transfer
        .group_id
        .as_deref()
        .filter(|group_id| !group_id.is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::ErrorClass;
    use crate::ledger::normalize::normalize;
    use crate::ledger::request::LedgerRequest;

    // Behaviour on small books. Each expectation is what the oracle's
    // `_prepare_rp2_asset_input` gives for the same rows: the gate
    // details, the entries' arguments, and the first failure
    // (`tests/test_tax_ledger_prepare.py` compares the two on every golden
    // and seeded generic book and on edge books like these, and checks each
    // failure against RP2). The Austrian cases set the Austrian fields of
    // the normalized stage by hand (a swap link, a move's regime), since
    // normalize does not derive them yet; the oracle gives the same gates
    // and notes for those values, beside the regimes its tracker adds.

    const JAN: &str = "2024-01-01T00:00:00Z";
    const FEB: &str = "2024-02-01T00:00:00Z";
    const MAR: &str = "2024-03-01T00:00:00Z";
    const APR: &str = "2024-04-01T00:00:00Z";
    const UNPARSED: &str = "not a time";

    /// A parsed timestamp at midnight UTC, or the one the builder writes for
    /// text it cannot parse.
    fn ts(raw: &str) -> Value {
        let seconds = match raw {
            JAN => 1_704_067_200_i64,
            FEB => 1_706_745_600,
            MAR => 1_709_251_200,
            APR => 1_711_929_600,
            _ => {
                return json!({"raw": raw, "error": format!("Error parsing parameter 'timestamp': Unknown string format: {raw}")})
            }
        };
        let date = &raw[..10];
        json!({
            "raw": raw,
            "us": seconds * 1_000_000,
            "offset_s": 0,
            "date": date,
            "year": 2024,
            "vienna_date": date,
            "display": format!("{date} 00:00:00+00:00"),
            "iso": format!("{date}T00:00:00+00:00"),
        })
    }

    /// A projection row: `id` is also its transaction (anchor) unless
    /// `fields` says otherwise.
    fn row(id: &str, wallet: &str, direction: &str, at: &str, amount: u64, fields: Value) -> Value {
        let mut row = json!({
            "id": format!("custody-tax:{id}"),
            "anchor": id,
            "asset": "BTC",
            "wallet_id": wallet,
            "direction": direction,
            "ts": ts(at),
            "amount_msat": amount,
            "fee_msat": 0,
            "rate": "40000E0",
            "value": null,
            "coarse": false,
            "kind": if direction == "inbound" { "deposit" } else { "withdrawal" },
            "text": id,
            "regime_override": null,
            "category_override": null,
            "taxability_off": false,
            "privacy_hop": false,
            "hold": null,
            "chain_order": null,
        });
        for (key, value) in fields.as_object().expect("fields") {
            row[key] = value.clone();
        }
        row
    }

    fn pair(out: &str, inbound: &str, group_id: Option<&str>) -> Value {
        json!({
            "out": format!("custody-tax:{out}"),
            "in": format!("custody-tax:{inbound}"),
            "source": "reviewed_custody_component",
            "group_id": group_id,
        })
    }

    fn request(country: Value, rows: Vec<Value>, pairs: Vec<Value>) -> LedgerRequest {
        serde_json::from_value(json!({
            "schema_version": 2,
            "operation": "prepare",
            "policy": {
                "country": country,
                "method": "fifo",
                "method_label": "FIFO",
                "require_coarse_review": false,
            },
            "config": {"assets": ["BTC"], "exchanges": ["Cold", "Exchange", "Hot"], "holders": ["Default"]},
            "holder": "Default",
            "wallets": [
                {"id": "A", "label": "Cold"},
                {"id": "B", "label": "Hot"},
                {"id": "C", "label": "Exchange"},
                {"id": "U", "label": null},
            ],
            "rows": rows,
            "intra_pairs": pairs,
            "cross_asset_pairs": [],
        }))
        .expect("a request")
    }

    fn generic(rows: Vec<Value>, pairs: Vec<Value>) -> LedgerRequest {
        request(
            json!({"kind": "generic", "long_term_days": 365}),
            rows,
            pairs,
        )
    }

    fn austrian(rows: Vec<Value>, pairs: Vec<Value>) -> LedgerRequest {
        request(json!({"kind": "at"}), rows, pairs)
    }

    /// Normalizes and walks a one-asset book, after `edit` changes the
    /// normalized stage (the Austrian fields the generic normalize leaves
    /// empty).
    fn walk_with(request: &LedgerRequest, edit: impl Fn(&mut Normalized)) -> EngineResult<Walked> {
        let book = Book::new(request).expect("a book");
        let [part] = book.assets.as_slice() else {
            panic!("one asset");
        };
        let mut normalized = normalize(&book, part, &part.rows).expect("normalized");
        edit(&mut normalized);
        walk(&book, part, &normalized, &request.config.spec())
    }

    fn walk_book(request: &LedgerRequest) -> EngineResult<Walked> {
        walk_with(request, |_| {})
    }

    /// Row ids without their prefix.
    fn name(request: &LedgerRequest, index: usize) -> &str {
        request.rows[index].id.trim_start_matches("custody-tax:")
    }

    fn quarantines(request: &LedgerRequest, walked: &Walked) -> Vec<Value> {
        walked
            .quarantines
            .iter()
            .map(|item| {
                assert_eq!(item.detail_kind, DetailKind::Plain);
                json!([
                    name(request, item.row),
                    item.reason.as_str(),
                    Value::Object(item.detail.clone()),
                ])
            })
            .collect()
    }

    /// An entry's kind, row number, row, type, and notes.
    type EntrySummary = (EntryKind, i64, String, Option<String>, Option<String>);

    /// `(kind, row, unique id, type, notes)` of each entry.
    fn entries(request: &LedgerRequest, walked: &Walked) -> Vec<EntrySummary> {
        walked
            .entries
            .as_ref()
            .expect("entries")
            .iter()
            .map(|entry| {
                let input = &entry.input;
                assert_eq!(
                    input.unique_id.as_deref(),
                    Some(request.rows[entry.row].id.as_str())
                );
                (
                    input.kind,
                    input.row,
                    name(request, entry.row).to_owned(),
                    input.transaction_type.clone(),
                    input.notes.clone(),
                )
            })
            .collect()
    }

    fn canonical(value: &Option<Dec>) -> Option<String> {
        value.as_ref().map(|value| value.0.to_canonical())
    }

    fn float_of(text: &str) -> Value {
        json!({ "float": text })
    }

    fn error_of(result: EngineResult<Walked>) -> (ErrorClass, String) {
        let error = result.expect_err("the walk fails");
        (error.class, error.message)
    }

    fn invalid_operation() -> (ErrorClass, String) {
        (
            ErrorClass::InvalidOperation,
            String::from("[<class 'decimal.InvalidOperation'>]"),
        )
    }

    #[test]
    fn entries_are_the_adapters_constructor_calls_in_walk_order() {
        let request = generic(
            vec![
                row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                row(
                    "earn",
                    "A",
                    "inbound",
                    JAN,
                    10_000_000_000,
                    json!({"kind": "income"}),
                ),
                row(
                    "move-out",
                    "A",
                    "outbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": "move", "fee_msat": 10_000_000}),
                ),
                row(
                    "move-in",
                    "B",
                    "inbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": "move", "text": null}),
                ),
                row(
                    "sell",
                    "B",
                    "outbound",
                    MAR,
                    20_000_000_000,
                    json!({"fee_msat": 1_000_000}),
                ),
                row(
                    "fee",
                    "A",
                    "outbound",
                    APR,
                    0,
                    json!({"fee_msat": 100_000_000}),
                ),
            ],
            vec![pair("move-out", "move-in", None)],
        );
        let walked = walk_book(&request).unwrap();
        assert!(walked.quarantines.is_empty());
        let some = |text: &str| Some(String::from(text));
        assert_eq!(
            entries(&request, &walked),
            vec![
                (EntryKind::In, 1, "buy".into(), some("BUY"), some("buy")),
                (
                    EntryKind::In,
                    2,
                    "earn".into(),
                    some("INCOME"),
                    some("earn")
                ),
                (EntryKind::Out, 4, "sell".into(), some("SELL"), some("sell")),
                (EntryKind::Out, 5, "fee".into(), some("FEE"), some("fee")),
                (
                    EntryKind::Intra,
                    3,
                    "move-out".into(),
                    None,
                    some("move-out")
                ),
            ]
        );
        let built = walked.entries.as_ref().unwrap();
        let buy = &built[0].input;
        assert_eq!(
            (
                buy.exchange.as_deref(),
                buy.holder.as_deref(),
                canonical(&buy.spot_price),
                canonical(&buy.crypto_in),
                canonical(&buy.crypto_fee),
                canonical(&buy.fiat_in_no_fee),
                canonical(&buy.fiat_in_with_fee),
                canonical(&buy.fiat_fee),
            ),
            (
                Some("Cold"),
                Some("Default"),
                some("40000E0"),
                some("1E0"),
                None,
                some("40000E0"),
                some("40000E0"),
                some("0E0"),
            )
        );
        let sell = &built[2].input;
        assert_eq!(
            (
                canonical(&sell.crypto_out_no_fee),
                canonical(&sell.crypto_fee),
                canonical(&sell.crypto_out_with_fee),
                canonical(&sell.fiat_out_no_fee),
                canonical(&sell.fiat_fee),
            ),
            // `fee * spot_price`, one CPython product.
            (
                some("2E-1"),
                some("1E-5"),
                None,
                some("80000E-1"),
                some("40000E-5")
            )
        );
        let fee = &built[3].input;
        assert_eq!(
            (canonical(&fee.fiat_out_no_fee), canonical(&fee.fiat_fee)),
            (None, some("40000E-3"))
        );
        let moved = &built[4].input;
        assert_eq!(
            (
                moved.from_exchange.as_deref(),
                moved.from_holder.as_deref(),
                moved.to_exchange.as_deref(),
                moved.to_holder.as_deref(),
                canonical(&moved.spot_price),
                canonical(&moved.crypto_sent),
                canonical(&moved.crypto_received),
            ),
            (
                Some("Cold"),
                Some("Default"),
                Some("Hot"),
                Some("Default"),
                some("40000E0"),
                some("5001E-4"),
                some("5E-1"),
            )
        );
        // Each entry carries its `str(transaction)` and its row's time.
        assert!(sell
            .text
            .starts_with("OutTransaction:\n  id=4\n  timestamp=2024-03-01 00:00:00.000000 +0000"));
        assert_eq!(moved.ts.raw, FEB);
        let audit = serde_json::to_value(&walked.audit).unwrap();
        assert_eq!(
            audit,
            json!([{
                "pair": 0,
                "sent": "5001E-4",
                "received": "5E-1",
                "fee": "1E-4",
                "sent_msat": 50_010_000_000_u64,
                "received_msat": 50_000_000_000_u64,
                "fee_msat": 10_000_000,
                "spot_price": "40000E0",
                "at_regime": null,
                "regime_flows": null,
                "at_pool": null,
                "description": "Transfer Cold -> Hot",
            }])
        );
    }

    #[test]
    fn an_unpriced_move_without_a_fee_is_priced_zero() {
        let request = generic(
            vec![
                row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                row(
                    "move-out",
                    "A",
                    "outbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": "move", "rate": null}),
                ),
                row(
                    "move-in",
                    "B",
                    "inbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": "move", "rate": null}),
                ),
            ],
            vec![pair("move-out", "move-in", Some("g"))],
        );
        let walked = walk_book(&request).unwrap();
        let moved = &walked.entries.as_ref().unwrap()[1].input;
        assert_eq!(canonical(&moved.spot_price).as_deref(), Some("0E0"));
        assert!(walked.audit[0].spot_price.is_none());
    }

    #[test]
    fn an_ungrouped_move_that_does_not_fit_is_quarantined_once_per_transaction() {
        let rows = |in_anchor: &str| {
            vec![
                row("buy", "A", "inbound", JAN, 10_000_000_000, json!({})),
                row(
                    "move-out",
                    "A",
                    "outbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": "move", "fee_msat": 10_000_000}),
                ),
                row(
                    "move-in",
                    "B",
                    "inbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": in_anchor}),
                ),
            ]
        };
        let failed = json!({"from_wallet": "Cold", "asset": "BTC", "required": float_of("5001E-4"), "available": float_of("1E-1")});
        let distinct = generic(rows("receipt"), vec![pair("move-out", "move-in", None)]);
        let walked = walk_book(&distinct).unwrap();
        let mut paired = failed.clone();
        paired["paired_leg"] = json!("inbound");
        paired["blocked_by_transaction_id"] = json!("move");
        assert_eq!(
            quarantines(&distinct, &walked),
            vec![
                json!(["move-out", "insufficient_lots", failed]),
                json!(["move-in", "insufficient_lots", paired]),
            ]
        );
        assert!(walked.audit.is_empty());
        // Both legs of one transaction: one quarantine.
        let shared = generic(rows("move"), vec![pair("move-out", "move-in", None)]);
        let walked = walk_book(&shared).unwrap();
        assert_eq!(
            quarantines(&shared, &walked),
            vec![json!(["move-out", "insufficient_lots", failed])]
        );
    }

    #[test]
    fn a_group_is_checked_whole_at_its_first_move_and_each_move_again_at_its_own() {
        let book = |second_amount: u64, second_anchor: &str| {
            generic(
                vec![
                    row("buy", "A", "inbound", JAN, 50_000_000_000, json!({})),
                    row(
                        "one-out",
                        "A",
                        "outbound",
                        FEB,
                        15_000_000_000,
                        json!({"anchor": "one", "fee_msat": 1_000_000}),
                    ),
                    row(
                        "one-in",
                        "B",
                        "inbound",
                        FEB,
                        15_000_000_000,
                        json!({"anchor": "one"}),
                    ),
                    row("spend", "A", "outbound", MAR, 30_000_000_000, json!({})),
                    row(
                        "two-out",
                        "A",
                        "outbound",
                        APR,
                        second_amount,
                        json!({"anchor": second_anchor, "fee_msat": 1_000_000}),
                    ),
                    row(
                        "two-in",
                        "C",
                        "inbound",
                        APR,
                        second_amount,
                        json!({"anchor": "receipt"}),
                    ),
                    row(
                        "three-out",
                        "B",
                        "outbound",
                        APR,
                        1_000_000_000,
                        json!({"anchor": "three"}),
                    ),
                    row(
                        "three-in",
                        "C",
                        "inbound",
                        APR,
                        1_000_000_000,
                        json!({"anchor": "three"}),
                    ),
                ],
                vec![
                    pair("one-out", "one-in", Some("g")),
                    pair("two-out", "two-in", Some("g")),
                    pair("three-out", "three-in", Some("g")),
                ],
            )
        };
        let flag = |from: &str, to: &str, inbound: bool| {
            json!({
                "asset": "BTC", "from_wallet": from, "to_wallet": to, "direction": "transfer",
                "paired_leg": inbound, "transfer_group_id": "g", "blocked_by_reason": "insufficient_lots",
            })
        };
        // The group fits at its first move; the spend then drains the
        // source, so the second move fails at its own position and the
        // group's later move is skipped. The first stays booked and is not
        // flagged.
        let drained = book(15_000_000_000, "two");
        let walked = walk_book(&drained).unwrap();
        assert_eq!(
            quarantines(&drained, &walked),
            vec![
                json!(["two-out", "insufficient_lots", {
                    "from_wallet": "Cold", "asset": "BTC", "required": float_of("15001E-5"),
                    "available": float_of("4999E-5"), "transfer_group_id": "g",
                }]),
                json!([
                    "two-in",
                    "derived_transfer_group_blocked",
                    flag("Cold", "Exchange", true)
                ]),
                json!([
                    "three-out",
                    "derived_transfer_group_blocked",
                    flag("Hot", "Exchange", false)
                ]),
            ]
        );
        let booked: Vec<_> = entries(&drained, &walked)
            .into_iter()
            .map(|(_, row, id, _, _)| (row, id))
            .collect();
        assert_eq!(
            booked,
            [
                (1, "buy".into()),
                (3, "spend".into()),
                (2, "one-out".into())
            ]
        );
        assert_eq!(walked.audit.len(), 1);

        // A group that does not fit at its first move books none of its
        // moves: the failing move first, each transaction named once (the
        // second move leaves from a slice of the first one's transaction).
        let short = book(40_000_000_000, "one");
        let walked = walk_book(&short).unwrap();
        assert_eq!(
            quarantines(&short, &walked),
            vec![
                json!(["two-out", "insufficient_lots", {
                    "from_wallet": "Cold", "asset": "BTC", "required": float_of("40001E-5"),
                    "available": float_of("34999E-5"), "transfer_group_id": "g",
                }]),
                json!([
                    "two-in",
                    "derived_transfer_group_blocked",
                    flag("Cold", "Exchange", true)
                ]),
                json!([
                    "three-out",
                    "derived_transfer_group_blocked",
                    flag("Hot", "Exchange", false)
                ]),
            ]
        );
        assert!(walked.audit.is_empty());
        assert_eq!(entries(&short, &walked).len(), 2);
    }

    #[test]
    fn inbound_kinds_that_look_like_unlisted_income_are_quarantined() {
        let rows = || {
            vec![
                row(
                    "reward",
                    "A",
                    "inbound",
                    JAN,
                    1_000,
                    json!({"kind": "staking_reward"}),
                ),
                row(
                    "staking",
                    "A",
                    "inbound",
                    JAN,
                    1_000,
                    json!({"kind": "staking"}),
                ),
                row(
                    "drop",
                    "A",
                    "inbound",
                    JAN,
                    1_000,
                    json!({"kind": "airdrop"}),
                ),
                row(
                    "fork",
                    "A",
                    "inbound",
                    JAN,
                    1_000,
                    json!({"kind": "hard_fork"}),
                ),
                row(
                    "wages",
                    "A",
                    "inbound",
                    JAN,
                    1_000,
                    json!({"kind": "wages"}),
                ),
                // Listed, so it books though it reads like a reward.
                row(
                    "mined",
                    "A",
                    "inbound",
                    JAN,
                    1_000,
                    json!({"kind": "mining_reward"}),
                ),
            ]
        };
        let book = generic(rows(), Vec::new());
        let walked = walk_book(&book).unwrap();
        assert_eq!(
            quarantines(&book, &walked),
            vec![json!(["reward", "unclassified_income_kind", {
                "wallet": "Cold", "asset": "BTC", "direction": "inbound", "kind": "staking_reward",
            }])]
        );
        let types: Vec<_> = entries(&book, &walked)
            .into_iter()
            .map(|(_, _, id, kind, _)| (id, kind.unwrap()))
            .collect();
        assert_eq!(
            types,
            [
                ("staking".into(), "STAKING".into()),
                ("drop".into(), "AIRDROP".into()),
                ("fork".into(), "HARDFORK".into()),
                ("wages".into(), "BUY".into()),
                ("mined".into(), "MINING".into()),
            ]
        );
        // An Austrian book cannot value airdrops and hard forks.
        let book = austrian(rows(), Vec::new());
        let walked = walk_book(&book).unwrap();
        let reasons: Vec<_> = walked
            .quarantines
            .iter()
            .map(|item| (name(&book, item.row), item.reason.as_str()))
            .collect();
        assert_eq!(
            reasons,
            [
                ("reward", "unclassified_income_kind"),
                ("drop", "acquisition_valuation_unsupported"),
                ("fork", "acquisition_valuation_unsupported"),
            ]
        );
        assert_eq!(
            walked.quarantines[1].detail,
            detail([
                ("wallet", Value::from("Cold")),
                ("asset", Value::from("BTC")),
                ("kind", Value::from("airdrop")),
                ("tax_country", Value::from("at")),
                ("valuation_mode", Value::from("zero_cost")),
                ("detail", Value::from("Austrian airdrop/hardfork valuation requires unsupported zero-cost semantics; a label does not prove eligibility.")),
            ])
        );
    }

    #[test]
    fn outbound_gates_hold_dispositions_shortfalls_and_tainted_lots() {
        let request = generic(
            vec![
                row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                row(
                    "early-sale",
                    "A",
                    "outbound",
                    JAN,
                    10_000_000_000,
                    json!({}),
                ),
                // A gift keeps its coins in the account and taints the lots
                // from its time.
                row(
                    "gift",
                    "A",
                    "outbound",
                    FEB,
                    80_000_000_000,
                    json!({"kind": "gift"}),
                ),
                row("too-much", "A", "outbound", FEB, 95_000_000_000, json!({})),
                row(
                    "tainted",
                    "A",
                    "outbound",
                    FEB,
                    10_000_000_000,
                    json!({"fee_msat": 1_000}),
                ),
                row("later", "A", "outbound", MAR, 10_000_000_000, json!({})),
            ],
            Vec::new(),
        );
        let walked = walk_book(&request).unwrap();
        assert_eq!(
            quarantines(&request, &walked),
            vec![
                json!(["gift", "non_sale_disposal_kind", {"wallet": "Cold", "asset": "BTC", "direction": "outbound", "kind": "gift"}]),
                json!(["too-much", "insufficient_lots", {
                    "wallet": "Cold", "asset": "BTC", "required": float_of("95E-2"), "available": float_of("9E-1"),
                }]),
                json!(["tainted", "basis_provenance_incomplete", {
                    "wallet": "Cold", "asset": "BTC", "required": float_of("10000001E-8"), "lot_state_uncertain_since": FEB,
                }]),
                json!(["later", "basis_provenance_incomplete", {
                    "wallet": "Cold", "asset": "BTC", "required": float_of("1E-1"), "lot_state_uncertain_since": FEB,
                }]),
            ]
        );
        let booked: Vec<_> = entries(&request, &walked)
            .into_iter()
            .map(|(_, _, id, _, _)| id)
            .collect();
        assert_eq!(booked, ["buy", "early-sale"]);
    }

    #[test]
    fn dropped_inbounds_taint_the_lots_from_their_time() {
        let rows = |kind: &str| {
            vec![
                row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                row("sale", "A", "outbound", FEB, 10_000_000_000, json!({})),
                row("drop", "A", "inbound", MAR, 1_000, json!({"kind": kind})),
                row("late", "A", "outbound", APR, 10_000_000_000, json!({})),
            ]
        };
        let tainted = |request: &LedgerRequest| -> Vec<String> {
            walk_book(request)
                .unwrap()
                .quarantines
                .iter()
                .filter(|item| item.reason == QuarantineReason::BasisProvenanceIncomplete)
                .map(|item| name(request, item.row).to_owned())
                .collect()
        };
        assert_eq!(
            tainted(&generic(rows("referral_bonus"), Vec::new())),
            ["late"]
        );
        // Airdrops taint only an Austrian book's lots.
        assert!(tainted(&generic(rows("airdrop"), Vec::new())).is_empty());
        assert_eq!(tainted(&austrian(rows("airdrop"), Vec::new())), ["late"]);
        // A listed income kind books and taints nothing.
        assert!(tainted(&generic(rows("mining"), Vec::new())).is_empty());
    }

    #[test]
    fn a_marked_swap_out_skips_the_gates_and_can_overdraw() {
        let request = austrian(
            vec![
                row("buy-a", "A", "inbound", JAN, 40_000_000_000, json!({})),
                row("buy-b", "B", "inbound", JAN, 100_000_000_000, json!({})),
                row("swap-out", "A", "outbound", FEB, 50_000_000_000, json!({})),
                row("sale", "B", "outbound", MAR, 100_000_000_000, json!({})),
            ],
            Vec::new(),
        );
        let link = |normalized: &mut Normalized| {
            normalized.events[2].at_swap_link = Some(String::from("swap-1"));
        };
        let walked = walk_with(&request, link).unwrap();
        // The overdraw leaves the priced amount short of the next sale,
        // though its own account holds it.
        assert_eq!(
            quarantines(&request, &walked),
            vec![json!(["sale", "missing_cost_basis", {
                "wallet": "Hot", "asset": "BTC", "required": float_of("1E0"), "priced_available": float_of("9E-1"),
            }])]
        );
        let notes: Vec<_> = entries(&request, &walked)
            .into_iter()
            .map(|(_, _, id, _, notes)| (id, notes.unwrap()))
            .collect();
        assert_eq!(
            notes,
            [
                ("buy-a".into(), "at_pool=default buy-a".into()),
                ("buy-b".into(), "at_pool=default buy-b".into()),
                (
                    "swap-out".into(),
                    "at_pool=default at_swap_link=swap-1 swap-out".into()
                ),
            ]
        );
        // Unmarked, the swap-out fails its account's gate.
        let walked = walk_book(&request).unwrap();
        let reasons: Vec<_> = walked
            .quarantines
            .iter()
            .map(|item| (name(&request, item.row), item.reason.as_str()))
            .collect();
        assert_eq!(reasons, [("swap-out", "insufficient_lots")]);

        // A move's fee leaves the priced amount too, so after the overdraw
        // a sale its account covers finds too little priced.
        let request = austrian(
            vec![
                row("buy-a", "A", "inbound", JAN, 40_000_000_000, json!({})),
                row("buy-b", "B", "inbound", JAN, 60_000_000_000, json!({})),
                row(
                    "move-out",
                    "B",
                    "outbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": "move", "fee_msat": 5_000_000_000_u64}),
                ),
                row(
                    "move-in",
                    "C",
                    "inbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": "move"}),
                ),
                row("swap-out", "A", "outbound", MAR, 50_000_000_000, json!({})),
                row("sale", "C", "outbound", APR, 48_000_000_000, json!({})),
            ],
            vec![pair("move-out", "move-in", None)],
        );
        let walked = walk_with(&request, |normalized| {
            normalized.events[2].at_swap_link = Some(String::from("swap-1"));
        })
        .unwrap();
        assert_eq!(
            quarantines(&request, &walked),
            vec![json!(["sale", "missing_cost_basis", {
                "wallet": "Exchange", "asset": "BTC", "required": float_of("48E-2"),
                "priced_available": float_of("45E-2"),
            }])]
        );
    }

    #[test]
    fn an_asset_without_an_inbound_entry_hands_the_lot_engine_nothing() {
        let request = austrian(
            vec![row(
                "swap-out",
                "A",
                "outbound",
                FEB,
                50_000_000_000,
                json!({}),
            )],
            Vec::new(),
        );
        let walked = walk_with(&request, |normalized| {
            normalized.events[0].at_swap_link = Some(String::from("swap-1"));
        })
        .unwrap();
        assert!(walked.quarantines.is_empty());
        assert!(walked.entries.is_none());
    }

    #[test]
    fn austrian_moves_carry_the_pool_in_their_notes_and_audit() {
        let request = austrian(
            vec![
                row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                row(
                    "move-out",
                    "A",
                    "outbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": "move", "fee_msat": 1_000_000}),
                ),
                row(
                    "move-in",
                    "B",
                    "inbound",
                    FEB,
                    50_000_000_000,
                    json!({"anchor": "move"}),
                ),
            ],
            vec![pair("move-out", "move-in", None)],
        );
        let walked = walk_with(&request, |normalized| {
            normalized.transfers[0].at_regime = Some(Regime::Neu);
        })
        .unwrap();
        let moved = &walked.entries.as_ref().unwrap()[1].input;
        assert_eq!(
            moved.notes.as_deref(),
            Some("at_regime=neu at_pool=default move-out")
        );
        let audit = &walked.audit[0];
        assert_eq!(
            (
                audit.at_regime,
                audit.at_pool.as_deref(),
                audit.description.as_str()
            ),
            (
                Some(Regime::Neu),
                Some("default"),
                "at_regime=neu at_pool=default Transfer Cold -> Hot"
            )
        );
    }

    #[test]
    fn infinite_values_fail_where_rps_constructor_compares_them() {
        let funded = |sale: Value| {
            generic(
                vec![
                    row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                    sale,
                ],
                Vec::new(),
            )
        };
        // An inbound price is compared first, after the time is parsed.
        let price = generic(
            vec![row(
                "buy",
                "A",
                "inbound",
                JAN,
                100_000_000_000,
                json!({"rate": "Infinity"}),
            )],
            Vec::new(),
        );
        assert_eq!(error_of(walk_book(&price)), invalid_operation());
        let untimed = generic(
            vec![row(
                "buy",
                "A",
                "inbound",
                UNPARSED,
                100_000_000_000,
                json!({"rate": "Infinity"}),
            )],
            Vec::new(),
        );
        let unparsed = (
            ErrorClass::ValueError,
            String::from("Error parsing parameter 'timestamp': Unknown string format: not a time"),
        );
        assert_eq!(error_of(walk_book(&untimed)), unparsed);
        // An infinite value is compared after the wallet's label.
        let value = generic(
            vec![row(
                "buy",
                "U",
                "inbound",
                JAN,
                100_000_000_000,
                json!({"value": "Infinity"}),
            )],
            Vec::new(),
        );
        assert_eq!(
            error_of(walk_book(&value)),
            (
                ErrorClass::TypeError,
                String::from("Parameter 'exchange' has non-string value None")
            )
        );
        // A sale's `fee * spot_price` is an argument, evaluated before the
        // constructor: without a fee it is `0 * Infinity`.
        let no_fee = funded(row(
            "sale",
            "A",
            "outbound",
            UNPARSED,
            10_000_000_000,
            json!({"rate": "Infinity"}),
        ));
        assert_eq!(error_of(walk_book(&no_fee)), invalid_operation());
        let with_fee = funded(row(
            "sale",
            "A",
            "outbound",
            UNPARSED,
            10_000_000_000,
            json!({"rate": "Infinity", "fee_msat": 1_000}),
        ));
        assert_eq!(error_of(walk_book(&with_fee)), unparsed);
        // The fee's value overflows while the sale's own does not.
        let overflow = generic(
            vec![
                row("buy", "A", "inbound", JAN, 400_000_000_000, json!({})),
                row(
                    "sale",
                    "A",
                    "outbound",
                    UNPARSED,
                    1,
                    json!({"rate": "9E999999", "fee_msat": 200_000_000_000_u64}),
                ),
            ],
            Vec::new(),
        );
        assert_eq!(
            error_of(walk_book(&overflow)),
            (
                ErrorClass::Overflow,
                String::from("[<class 'decimal.Overflow'>]")
            )
        );
        let value = funded(row(
            "sale",
            "A",
            "outbound",
            FEB,
            10_000_000_000,
            json!({"value": "Infinity"}),
        ));
        assert_eq!(error_of(walk_book(&value)), invalid_operation());
        // A move compares its price before its wallets.
        let moves = |rate: &str| {
            generic(
                vec![
                    row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                    row(
                        "move-out",
                        "A",
                        "outbound",
                        FEB,
                        10_000_000_000,
                        json!({"anchor": "move", "rate": rate}),
                    ),
                    row(
                        "move-in",
                        "U",
                        "inbound",
                        FEB,
                        10_000_000_000,
                        json!({"anchor": "move", "rate": rate}),
                    ),
                ],
                vec![pair("move-out", "move-in", None)],
            )
        };
        assert_eq!(error_of(walk_book(&moves("Infinity"))), invalid_operation());
        assert_eq!(
            error_of(walk_book(&moves("40000E0"))),
            (
                ErrorClass::TypeError,
                String::from("Parameter 'to_exchange' has non-string value None")
            )
        );
    }

    #[test]
    fn the_holder_is_the_profile_label_as_it_is() {
        let mut request = generic(
            vec![row("buy", "A", "inbound", JAN, 100_000_000_000, json!({}))],
            Vec::new(),
        );
        request.holder = Some(String::from(" Default "));
        assert_eq!(
            error_of(walk_book(&request)),
            (
                ErrorClass::ValueError,
                String::from("Parameter 'holder' value is not known:  Default ")
            )
        );
    }
}
