//! One asset's rows and moves, normalized for the walk (design §4.4).
//!
//! This is the part of the adapter's `normalize_tax_asset_inputs`
//! (`tests/tax_oracle/normalize.py:1570-2692`) a finalized projection
//! reaches, ported literally. The request builder refuses every shape that
//! would reach the rest (`ledger_request.INVARIANTS`), so the dead branches
//! have no port.
//!
//! On an Austrian book the regime tracker ([`super::regimes`]) runs first,
//! over the pass's rows without a hold and its moves without a held leg
//! (`:1775-1866`).
//!
//! The pass visits its rows in projection order:
//!
//! - a held row (a conflicting spend, an unconfirmed one) is quarantined, and
//!   a move with a held leg is not a move: its other leg is an event
//!   (`:1775-1784`, `:1936-1966`);
//! - a move is handled at the first of its legs the pass visits
//!   (`:2061-2067`). It sends the out leg's amount and fee and receives the
//!   in leg's amount; the difference is its fee. Its price is the out leg's,
//!   else the in leg's (`:2288-2296`). A fee-bearing move whose price is
//!   coarse while the profile requires coarse prices to be reviewed, or that
//!   has no price, is quarantined together with its partner leg, and blocks
//!   its atomic group (`:182-222`, `:2301-2358`);
//! - every other row is an event (`:2412-2552`). A privacy hop is
//!   quarantined; so is coarse pricing that must be reviewed. An inbound row
//!   needs a price, and an outbound one a price for its amount, else for its
//!   fee.
//!
//! On an Austrian book an event's regime is its time's for an inbound row
//! and the tracker's for an outbound one, which may record an election
//! (Wahlrecht); a move's is the tracker's for its out leg, with the msat each
//! regime moved. A row's regime override replaces either and drops the
//! election (`:2366-2373`, `:2399-2401`, `:2503-2528`). An event whose row
//! the pass's swap links name ([`super::swaplinks`]) carries that link
//! (`:2526-2528`). Every pool is the global one.
//!
//! Then a blocked group loses its moves and its legs whose transactions no
//! quarantine names yet are flagged (`:2554-2581`), and the asset's lot
//! state is tainted from the earliest `occurred_at` of a visited row whose
//! transaction any of these quarantines names (`:2662-2691`).
//!
//! Values are CPython's: an amount is `Decimal(msat) / 10**11`, a price or a
//! fallback value one 32-digit operation, and every comparison exact. A
//! stored rate or value can be non-finite text, which behaves as CPython's
//! `Decimal` does where it is read: comparing a NaN raises
//! `InvalidOperation`, and `Infinity` is a price.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::decimal::{Decimal, DecimalError};
use crate::error::{EngineResult, ErrorClass};
use crate::manifest::{DetailKind, QuarantineReason, REGIME_BASIS_ELECTION};
use crate::num;

use super::regimes::{self, Tracked};
use super::request::{Country, DecimalText, Direction, Hold, Regime, Row, Ts};
use super::response::{Fiat, RegimeFlows};
use super::run::{AssetRows, Book, MovePair};

/// `tax_policy.GLOBAL_COST_BASIS_POOL_ID`: every row's cost-basis pool,
/// since no country allows another scope (the builder raises the policy's
/// own error for one).
pub(crate) const GLOBAL_POOL: &str = "global";

/// msat per BTC (`kassiber.msat.MSAT_PER_BTC`).
const MSAT_PER_BTC: u64 = 100_000_000_000;

/// An event the walk books: an acquisition, income, or disposal
/// (`NormalizedTaxEvent`).
#[derive(Clone, Debug)]
pub(crate) struct Event {
    /// The row.
    pub row: usize,
    pub wallet_label: Option<String>,
    pub amount: Decimal,
    pub fee: Decimal,
    pub spot_price: Fiat,
    pub fiat_value: Fiat,
    /// The row's text, else its id.
    pub description: String,
    /// Austrian books only.
    pub at_regime: Option<Regime>,
    /// Austrian books only: why the disposal has its regime.
    pub at_regime_basis: Option<&'static str>,
    pub cost_basis_pool_id: &'static str,
    /// Austrian books only: the swap the row is a leg of.
    pub at_swap_link: Option<String>,
}

/// A move the walk books (`NormalizedTaxTransfer`).
#[derive(Clone, Debug)]
pub(crate) struct Transfer {
    /// The move: its index in `intra_pairs`, and its legs.
    pub pair: usize,
    pub out: usize,
    pub inbound: usize,
    pub from_wallet_label: Option<String>,
    pub to_wallet_label: Option<String>,
    pub sent: Decimal,
    pub received: Decimal,
    pub fee: Decimal,
    pub spot_price: Option<Fiat>,
    /// The out leg's text, else `Transfer <from> -> <to>`.
    pub description: String,
    pub from_cost_basis_pool_id: &'static str,
    pub to_cost_basis_pool_id: &'static str,
    /// Austrian books only: the regime of the move's fee.
    pub at_regime: Option<Regime>,
    /// Austrian books only: the msat each regime moved.
    pub regime_flows: Option<RegimeFlows>,
    pub group_id: Option<String>,
}

/// One item of the walk's order: an event at its row, a move at its out leg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Event(usize),
    Transfer(usize),
}

/// A quarantine: the row whose transaction it names, and its typed detail.
#[derive(Clone, Debug)]
pub(crate) struct Quarantine {
    pub row: usize,
    pub reason: QuarantineReason,
    pub detail_kind: DetailKind,
    pub detail: Map<String, Value>,
}

/// One asset, normalized (`NormalizedTaxAssetInputs`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Normalized {
    pub events: Vec<Event>,
    pub transfers: Vec<Transfer>,
    /// Events and moves together, in the order normalize met them.
    pub ordered_items: Vec<Item>,
    pub quarantines: Vec<Quarantine>,
    /// `str(occurred_at)` of the earliest row a quarantine taints.
    pub earliest_lot_contamination_at: Option<String>,
}

/// `msat_to_btc`: `Decimal(msat) / Decimal("100000000000")`. Wide enough
/// for an amount plus its fee, as Python's integers are.
pub(crate) fn btc(msat: u128) -> EngineResult<Decimal> {
    num::div(&Decimal::from(msat), &Decimal::from(MSAT_PER_BTC))
}

/// `value > 0` for a stored decimal, as CPython evaluates it: the value when
/// it holds, `None` when not. Comparing a NaN, quiet or signaling, raises
/// `InvalidOperation`.
fn positive(value: &DecimalText) -> EngineResult<Option<Fiat>> {
    match value {
        DecimalText::Finite(value) => Ok(value.is_positive().then(|| Fiat::Finite(value.clone()))),
        DecimalText::NonFinite(text) => match text.as_str() {
            "Infinity" => Ok(Some(Fiat::Infinity)),
            "-Infinity" => Ok(None),
            _ => Err(DecimalError::InvalidOperation.into()),
        },
    }
}

/// `_spot_price_from_row` (`:370-385`): the stored rate when positive, else
/// the stored value over `quantity` when both are positive.
fn spot_price(row: &Row, quantity: &Decimal) -> EngineResult<Option<Fiat>> {
    if let Some(rate) = &row.rate {
        if let Some(rate) = positive(rate)? {
            return Ok(Some(rate));
        }
    }
    // `value > 0` is evaluated only for a positive quantity.
    if let (Some(value), true) = (&row.value, quantity.is_positive()) {
        if let Some(value) = positive(value)? {
            return Ok(Some(match value {
                Fiat::Finite(value) => Fiat::Finite(num::div(&value, quantity)?),
                Fiat::Infinity => Fiat::Infinity,
            }));
        }
    }
    Ok(None)
}

/// `_positive_fiat_value(row, amount * spot)` (`:396-413`): the stored
/// value when positive, else the product. Python computes the product
/// first, as the call's argument, so it raises even where the stored value
/// replaces it.
fn fiat_value(row: &Row, amount: &Decimal, spot: &Fiat) -> EngineResult<Fiat> {
    let fallback = match spot {
        Fiat::Finite(spot) => Fiat::Finite(num::mul(amount, spot)?),
        // `0 * Infinity` is an invalid operation; amounts are never negative.
        Fiat::Infinity if amount.is_zero() => return Err(DecimalError::InvalidOperation.into()),
        Fiat::Infinity => Fiat::Infinity,
    };
    if let Some(value) = &row.value {
        if let Some(value) = positive(value)? {
            return Ok(value);
        }
    }
    Ok(fallback)
}

/// Whether Python's `row["occurred_at"]` is truthy. A projection row's
/// timestamp is text (`str(... or "")` upstream), so only an empty one is
/// falsy; of the values that are not text, `None` is the falsy one a row
/// could carry.
pub(crate) fn occurred_at_is_set(ts: &Ts) -> bool {
    match ts {
        Ts::Parsed(_) => true,
        Ts::Failed(failed) => {
            !(failed.raw.is_empty()
                || (failed.error_class == Some(ErrorClass::TypeError) && failed.raw == "None"))
        }
    }
}

fn label_value(label: Option<&str>) -> Value {
    label.map_or(Value::Null, Value::from)
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Inbound => "inbound",
        Direction::Outbound => "outbound",
    }
}

/// A detail object from its fields; the reader serializes it with sorted
/// keys, so their order does not matter.
fn detail<const N: usize>(fields: [(&str, Value); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

/// `_with_transfer_group` (`:255-261`): a move's detail names its group.
fn with_group(mut detail: Map<String, Value>, group_id: Option<&str>) -> Map<String, Value> {
    if let Some(group_id) = group_id {
        detail.insert(String::from("transfer_group_id"), Value::from(group_id));
    }
    detail
}

/// One normalize pass over an asset's `active` rows (all of them unless a
/// pass excludes some), with the pass's swap links by row
/// (`at_swap_link_by_row_id`; read on Austrian books only). The holds come
/// from all of the asset's rows (the builder detects them so), and the
/// moves are all of the asset's moves, as the adapter passes them to every
/// pass (`adapter.py:1690-1703`).
pub(crate) fn normalize<'a>(
    book: &Book<'a>,
    part: &AssetRows<'a>,
    active: &[usize],
    links: &HashMap<usize, String>,
) -> EngineResult<Normalized> {
    Normalizer::new(book, part, links).run(active)
}

struct Normalizer<'b, 'a> {
    book: &'b Book<'a>,
    rows: &'a [Row],
    asset: &'a str,
    review_coarse: bool,
    austrian: bool,
    /// Moves without a held leg, in request order; by leg; with a group, by
    /// group (`:1775-1784`, `:1881-1893`).
    moves: Vec<&'b MovePair<'a>>,
    move_by_row: HashMap<usize, &'b MovePair<'a>>,
    moves_by_group: HashMap<&'a str, Vec<&'b MovePair<'a>>>,
    out: Normalized,
    /// Groups a failed move blocks, with the first reason, in the order
    /// they were first blocked.
    blocked: Vec<(&'a str, QuarantineReason)>,
    /// The regime tracker's findings (Austrian books only).
    tracked: Tracked,
    /// The pass's swap links, by row.
    links: &'b HashMap<usize, String>,
}

impl<'b, 'a> Normalizer<'b, 'a> {
    fn new(book: &'b Book<'a>, part: &'b AssetRows<'a>, links: &'b HashMap<usize, String>) -> Self {
        let rows = book.rows;
        let mut moves = Vec::new();
        let mut move_by_row = HashMap::new();
        let mut moves_by_group: HashMap<&'a str, Vec<&'b MovePair<'a>>> = HashMap::new();
        for pair in &part.pairs {
            if rows[pair.out].hold.is_some() || rows[pair.inbound].hold.is_some() {
                continue;
            }
            moves.push(pair);
            // Moves share no leg (the request guarantees it), so each row has
            // at most one.
            move_by_row.insert(pair.out, pair);
            move_by_row.insert(pair.inbound, pair);
            if let Some(group_id) = pair.group_id {
                moves_by_group.entry(group_id).or_default().push(pair);
            }
        }
        Normalizer {
            book,
            rows,
            asset: part.asset,
            review_coarse: book.request.policy.require_coarse_review,
            austrian: matches!(book.request.policy.country, Country::At {}),
            moves,
            move_by_row,
            moves_by_group,
            out: Normalized::default(),
            blocked: Vec::new(),
            tracked: Tracked::default(),
            links,
        }
    }

    fn run(mut self, active: &[usize]) -> EngineResult<Normalized> {
        if self.austrian {
            let order = regimes::regime_rows(self.rows, active, |row| occurred_at_is_set(&row.ts));
            self.tracked = regimes::track(self.rows, &order, &self.moves)?;
        }
        let mut handled = HashSet::new();
        for &index in active {
            if let Some(hold) = self.rows[index].hold {
                self.hold(index, hold);
                continue;
            }
            if let Some(&pair) = self.move_by_row.get(&index) {
                if handled.insert(pair.index) {
                    self.transfer(pair)?;
                }
                continue;
            }
            self.event(index)?;
        }
        self.block_groups();
        self.out.earliest_lot_contamination_at = self.contamination(active);
        Ok(self.out)
    }

    fn label(&self, row: usize) -> Option<&'a str> {
        self.book.wallet_label(&self.rows[row].wallet_id)
    }

    fn quarantine(
        &mut self,
        row: usize,
        reason: QuarantineReason,
        detail_kind: DetailKind,
        detail: Map<String, Value>,
    ) {
        self.out.quarantines.push(Quarantine {
            row,
            reason,
            detail_kind,
            detail,
        });
    }

    /// `blocked_transfer_group_reasons.setdefault(group_id, reason)`.
    fn block(&mut self, group_id: Option<&'a str>, reason: QuarantineReason) {
        if let Some(group_id) = group_id {
            if !self.blocked.iter().any(|(blocked, _)| *blocked == group_id) {
                self.blocked.push((group_id, reason));
            }
        }
    }

    /// `:1936-1966`; the reader adds the row's external id.
    fn hold(&mut self, index: usize, hold: Hold) {
        let mut fields = detail([
            ("wallet", label_value(self.label(index))),
            ("asset", Value::from(self.asset)),
            (
                "direction",
                Value::from(direction_name(self.rows[index].direction)),
            ),
        ]);
        let reason = match hold {
            Hold::ConflictingSpend => QuarantineReason::ConflictingSpend,
            Hold::PendingOnchainConfirmation => {
                fields.insert(
                    String::from("required_for"),
                    Value::from("confirmed_chain_history"),
                );
                QuarantineReason::PendingOnchainConfirmation
            }
        };
        self.quarantine(index, reason, DetailKind::Hold, fields);
    }

    /// `_quarantine_partner_leg` (`:182-222`): the leg the move's own
    /// quarantine does not name. The legs are distinct rows.
    fn partner_leg(&mut self, primary: usize, pair: &MovePair<'a>, reason: QuarantineReason) {
        let partner = if primary == pair.out {
            pair.inbound
        } else {
            pair.out
        };
        let fields = detail([
            ("from_wallet", label_value(self.label(pair.out))),
            ("to_wallet", label_value(self.label(pair.inbound))),
            ("asset", Value::from(self.asset)),
            ("direction", Value::from("transfer")),
            ("paired_leg", Value::from(true)),
        ]);
        self.quarantine(
            partner,
            reason,
            DetailKind::PartnerLeg,
            with_group(fields, pair.group_id),
        );
    }

    /// The pair branch (`:2061-2410`), live part.
    fn transfer(&mut self, pair: &MovePair<'a>) -> EngineResult<()> {
        let rows = self.rows;
        let (out_row, in_row) = (&rows[pair.out], &rows[pair.inbound]);
        // `outbound_transfer_evidence_msat` (no row includes its fee in its
        // amount) and `clamped_receipt_msat`. The legs' amounts are equal,
        // so nothing is clamped, nothing more is received than sent
        // (`transfer_mismatch`, `:2113-2138`), and no outflow is unrecognized
        // (`transfer_fee_implausible`, `:2213-2287`).
        let sent = btc(u128::from(out_row.amount_msat) + u128::from(out_row.fee_msat))?;
        let received = btc(in_row.amount_msat.into())?;
        let fee = num::sub(&sent, &received)?;
        let (spot, priced) = match spot_price(out_row, &btc(out_row.amount_msat.into())?)? {
            Some(spot) => (Some(spot), pair.out),
            None => (
                spot_price(in_row, &btc(in_row.amount_msat.into())?)?,
                pair.inbound,
            ),
        };
        if fee.is_positive() && spot.is_some() && rows[priced].coarse && self.review_coarse {
            let reason = QuarantineReason::PricingReviewRequired;
            self.block(pair.group_id, reason);
            // `_pricing_review_detail` of the priced leg; the reader adds
            // its pricing fields.
            let fields = detail([
                ("wallet", label_value(self.label(priced))),
                ("asset", Value::from(self.asset)),
                ("direction", Value::from("transfer")),
                ("required_for", Value::from("pricing_review")),
            ]);
            self.quarantine(
                priced,
                reason,
                DetailKind::PricingReview,
                with_group(fields, pair.group_id),
            );
            self.partner_leg(priced, pair, reason);
            return Ok(());
        }
        if spot.is_none() && fee.is_positive() {
            let reason = QuarantineReason::MissingSpotPrice;
            self.block(pair.group_id, reason);
            let fields = detail([
                ("from_wallet", label_value(self.label(pair.out))),
                ("to_wallet", label_value(self.label(pair.inbound))),
                ("asset", Value::from(self.asset)),
                ("direction", Value::from("transfer")),
                ("required_for", Value::from("transfer_fee")),
            ]);
            self.quarantine(
                pair.out,
                reason,
                DetailKind::Plain,
                with_group(fields, pair.group_id),
            );
            self.partner_leg(pair.out, pair, reason);
            return Ok(());
        }
        let from_label = self.label(pair.out);
        let to_label = self.label(pair.inbound);
        // The out leg's override, else the tracker's regime, which every move
        // without a held leg has; no time fallback (`:2366-2373`).
        let (at_regime, regime_flows) = if self.austrian {
            (
                out_row
                    .regime_override
                    .or_else(|| self.tracked.regimes.get(&pair.out).copied()),
                self.tracked.flows.get(&pair.index).cloned(),
            )
        } else {
            (None, None)
        };
        // An f-string writes a missing label as `None`.
        let description = out_row.text.clone().unwrap_or_else(|| {
            format!(
                "Transfer {} -> {}",
                from_label.unwrap_or("None"),
                to_label.unwrap_or("None")
            )
        });
        self.out.transfers.push(Transfer {
            pair: pair.index,
            out: pair.out,
            inbound: pair.inbound,
            from_wallet_label: from_label.map(str::to_owned),
            to_wallet_label: to_label.map(str::to_owned),
            sent,
            received,
            fee,
            spot_price: spot,
            description,
            from_cost_basis_pool_id: GLOBAL_POOL,
            to_cost_basis_pool_id: GLOBAL_POOL,
            at_regime,
            regime_flows,
            group_id: pair.group_id.map(str::to_owned),
        });
        self.out.ordered_items.push(Item::Transfer(pair.out));
        Ok(())
    }

    /// A standalone row (`:2412-2552`), live part.
    fn event(&mut self, index: usize) -> EngineResult<()> {
        let rows = self.rows;
        let row = &rows[index];
        let amount = btc(row.amount_msat.into())?;
        let fee = btc(row.fee_msat.into())?;
        let asset = self.asset;
        let direction = direction_name(row.direction);
        let label = self.label(index);
        let base = || {
            detail([
                ("wallet", label_value(label)),
                ("asset", Value::from(asset)),
                ("direction", Value::from(direction)),
            ])
        };
        if row.privacy_hop {
            // The reader adds the row's privacy evidence.
            self.quarantine(
                index,
                QuarantineReason::PrivacyHopUnresolved,
                DetailKind::PrivacyHop,
                base(),
            );
            return Ok(());
        }
        if row.coarse && self.review_coarse {
            let mut fields = base();
            fields.insert(String::from("required_for"), Value::from("pricing_review"));
            self.quarantine(
                index,
                QuarantineReason::PricingReviewRequired,
                DetailKind::PricingReview,
                fields,
            );
            return Ok(());
        }
        let (quantity, required_for) = match row.direction {
            Direction::Inbound => (&amount, "acquisition"),
            Direction::Outbound => {
                // `needed <= 0`: the builder refuses such a row, but the
                // skip costs nothing to keep.
                if !num::add(&amount, &fee)?.is_positive() {
                    return Ok(());
                }
                let quantity = if amount.is_positive() { &amount } else { &fee };
                (quantity, "disposal")
            }
        };
        let Some(spot) = spot_price(row, quantity)? else {
            let mut fields = base();
            fields.insert(String::from("required_for"), Value::from(required_for));
            self.quarantine(
                index,
                QuarantineReason::MissingSpotPrice,
                DetailKind::Plain,
                fields,
            );
            return Ok(());
        };
        let fiat_value = fiat_value(row, &amount, &spot)?;
        let (at_regime, at_regime_basis, at_swap_link) = if self.austrian {
            let (regime, basis) = self.event_regime(index)?;
            // `if linked:`, so an empty link is none.
            let link = self.links.get(&index).filter(|link| !link.is_empty());
            (regime, basis, link.cloned())
        } else {
            (None, None, None)
        };
        self.out.events.push(Event {
            row: index,
            wallet_label: label.map(str::to_owned),
            amount,
            fee,
            spot_price: spot,
            fiat_value,
            description: row.text.clone().unwrap_or_else(|| row.id.clone()),
            at_regime,
            at_regime_basis,
            cost_basis_pool_id: GLOBAL_POOL,
            at_swap_link,
        });
        self.out.ordered_items.push(Item::Event(index));
        Ok(())
    }

    /// An Austrian event's regime and why it has it (`:2503-2528`): an
    /// inbound row's time's regime; an outbound row's from the tracker,
    /// with its election. An override replaces the regime and drops the
    /// election. Python reads the time's regime for every event, also where
    /// the tracker has one; the tracker already read it, so this cannot
    /// fail first.
    fn event_regime(&self, index: usize) -> EngineResult<(Option<Regime>, Option<&'static str>)> {
        let row = &self.rows[index];
        let by_time = regimes::timestamp_regime(row)?;
        let (regime, basis) = match row.direction {
            Direction::Inbound => (by_time, None),
            Direction::Outbound => (
                self.tracked.regimes.get(&index).copied().unwrap_or(by_time),
                self.tracked
                    .elected
                    .contains(&index)
                    .then_some(REGIME_BASIS_ELECTION),
            ),
        };
        Ok(match row.regime_override {
            Some(chosen) => (Some(chosen), None),
            None => (Some(regime), basis),
        })
    }

    /// `:2554-2581` with `_append_group_block_quarantines` (`:264-308`): a
    /// blocked group books none of its moves, and each leg whose
    /// transaction no quarantine names yet is flagged once.
    fn block_groups(&mut self) {
        if self.blocked.is_empty() {
            return;
        }
        let blocked: HashSet<&str> = self.blocked.iter().map(|(group_id, _)| *group_id).collect();
        let removed: HashSet<usize> = self
            .out
            .transfers
            .iter()
            .filter(|transfer| {
                transfer
                    .group_id
                    .as_deref()
                    .is_some_and(|group_id| blocked.contains(group_id))
            })
            .map(|transfer| transfer.out)
            .collect();
        self.out
            .transfers
            .retain(|transfer| !removed.contains(&transfer.out));
        self.out
            .ordered_items
            .retain(|item| !matches!(item, Item::Transfer(out) if removed.contains(out)));
        let rows = self.rows;
        for (group_id, reason) in self.blocked.clone() {
            // `seen` starts from every quarantine so far, including the
            // ones earlier groups added.
            let mut seen: HashSet<&str> = self
                .out
                .quarantines
                .iter()
                .map(|quarantine| rows[quarantine.row].anchor.as_str())
                .collect();
            let pairs = self
                .moves_by_group
                .get(group_id)
                .cloned()
                .unwrap_or_default();
            for pair in pairs {
                for leg in [pair.out, pair.inbound] {
                    if !seen.insert(rows[leg].anchor.as_str()) {
                        continue;
                    }
                    let wallet = match self.label(leg) {
                        Some(label) if !label.is_empty() => label,
                        _ => rows[leg].wallet_id.as_str(),
                    };
                    let fields = detail([
                        ("asset", Value::from(self.asset)),
                        ("wallet", Value::from(wallet)),
                        ("direction", Value::from("transfer")),
                        ("transfer_group_id", Value::from(group_id)),
                        ("blocked_by_reason", Value::from(reason.as_str())),
                    ]);
                    self.quarantine(
                        leg,
                        QuarantineReason::DerivedTransferGroupBlocked,
                        DetailKind::Plain,
                        fields,
                    );
                }
            }
        }
    }

    /// `:2665-2691`: the earliest `occurred_at`, compared as text, among the
    /// visited rows whose transaction a quarantine names, so a quarantine on
    /// one slice taints every slice of its transaction.
    fn contamination(&self, active: &[usize]) -> Option<String> {
        let anchors: HashSet<&str> = self
            .out
            .quarantines
            .iter()
            .map(|quarantine| self.rows[quarantine.row].anchor.as_str())
            .collect();
        active
            .iter()
            .map(|&index| &self.rows[index])
            .filter(|row| anchors.contains(row.anchor.as_str()) && occurred_at_is_set(&row.ts))
            .map(|row| row.ts.raw())
            .min()
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ledger::request::{FailedTs, LedgerRequest};

    fn text(value: &str) -> DecimalText {
        serde_json::from_value(Value::from(value)).expect("decimal text")
    }

    fn dec(value: &str) -> Decimal {
        Decimal::from_canonical(value).expect("canonical decimal")
    }

    #[test]
    fn amounts_are_cpythons_msat_to_btc() {
        // `msat_to_btc(msat).as_tuple()`, printed by CPython 3.11.
        for (msat, expected) in [
            (0, "0E0"),
            (1, "1E-11"),
            (150_000_000_000, "15E-1"),
            (100_000_000_000, "1E0"),
            (123_456_789, "123456789E-11"),
            (u128::from(u64::MAX), "18446744073709551615E-11"),
            (2 * u128::from(u64::MAX), "3689348814741910323E-10"),
        ] {
            assert_eq!(btc(msat).unwrap().to_canonical(), expected, "{msat}");
        }
    }

    #[test]
    fn non_finite_values_compare_as_cpythons() {
        assert_eq!(positive(&text("Infinity")).unwrap(), Some(Fiat::Infinity));
        assert_eq!(positive(&text("-Infinity")).unwrap(), None);
        for nan in ["NaN", "-NaN", "sNaN", "-sNaN3", "NaN12"] {
            let error = positive(&text(nan)).unwrap_err();
            assert_eq!(error.class, ErrorClass::InvalidOperation, "{nan}");
            assert_eq!(error.message, "[<class 'decimal.InvalidOperation'>]");
        }
        assert_eq!(positive(&text("-0E0")).unwrap(), None);
        assert_eq!(positive(&text("0E-5")).unwrap(), None);
        assert_eq!(positive(&text("-1E0")).unwrap(), None);
        assert_eq!(
            positive(&text("1E-11")).unwrap(),
            Some(Fiat::Finite(dec("1E-11")))
        );
    }

    #[test]
    fn occurred_at_is_falsy_only_when_empty_or_none() {
        let failed = |raw: &str, error_class: Option<ErrorClass>| {
            Ts::Failed(FailedTs {
                raw: raw.to_owned(),
                error: String::from("x"),
                error_class,
            })
        };
        assert!(!occurred_at_is_set(&failed("", None)));
        assert!(!occurred_at_is_set(&failed(
            "None",
            Some(ErrorClass::TypeError)
        )));
        // The text "None" is truthy.
        assert!(occurred_at_is_set(&failed("None", None)));
        assert!(occurred_at_is_set(&failed("yesterday", None)));
    }

    // Behaviour on small books. Each expectation is what the oracle's
    // `normalize_tax_asset_inputs` gives for the same rows
    // (`tests/test_tax_ledger_prepare.py` compares the two on every golden
    // and seeded book without carries and on edge books like these; the
    // Austrian book below was replayed on the oracle's normalize).

    const JAN: &str = "2024-01-01T00:00:00Z";
    const FEB: &str = "2024-02-01T00:00:00Z";
    const MAR: &str = "2024-03-01T00:00:00Z";
    const APR: &str = "2024-04-01T00:00:00Z";
    /// Before the Neuvermögen cutoff.
    const ALT: &str = "2020-06-01T00:00:00Z";

    /// A parsed timestamp at midnight UTC, or a failed one (normalize reads
    /// only the text).
    fn ts(raw: &str) -> Value {
        let seconds = match raw {
            ALT => 1_590_969_600_i64,
            JAN => 1_704_067_200_i64,
            FEB => 1_706_745_600,
            MAR => 1_709_251_200,
            APR => 1_711_929_600,
            _ => {
                return json!({"raw": raw, "error": format!("Error parsing parameter 'timestamp': {raw}")})
            }
        };
        let date = &raw[..10];
        json!({
            "raw": raw,
            "us": seconds * 1_000_000,
            "offset_s": 0,
            "date": date,
            "year": raw[..4].parse::<i32>().expect("a year"),
            "vienna_date": date,
            "display": format!("{date} 00:00:00+00:00"),
            "iso": format!("{date}T00:00:00+00:00"),
        })
    }

    /// The row's time as the Austrian cutoff reads it.
    fn cutoff(raw: &str) -> Value {
        match ts(raw).get("us") {
            Some(us) => json!({ "us": us }),
            None => json!({ "error": format!("Invalid isoformat string: '{raw}'") }),
        }
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
            "cutoff_instant": cutoff(at),
            "amount_msat": amount,
            "fee_msat": 0,
            "rate": "40000E0",
            "value": null,
            "coarse": false,
            "kind": if direction == "inbound" { "deposit" } else { "withdrawal" },
            "text": null,
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

    fn request(rows: Vec<Value>, pairs: Vec<Value>, review_coarse: bool) -> LedgerRequest {
        book_request(
            json!({"kind": "generic", "long_term_days": 365}),
            rows,
            pairs,
            review_coarse,
        )
    }

    fn austrian(rows: Vec<Value>, pairs: Vec<Value>) -> LedgerRequest {
        book_request(json!({"kind": "at"}), rows, pairs, false)
    }

    fn book_request(
        country: Value,
        rows: Vec<Value>,
        pairs: Vec<Value>,
        review_coarse: bool,
    ) -> LedgerRequest {
        serde_json::from_value(json!({
            "schema_version": 2,
            "operation": "prepare",
            "policy": {
                "country": country,
                "method": "fifo",
                "method_label": "FIFO",
                "require_coarse_review": review_coarse,
            },
            "config": {"assets": ["BTC"], "exchanges": ["Cold", "Exchange", "Hot"], "holders": ["Default"]},
            "holder": "Default",
            "wallets": [
                {"id": "A", "label": "Cold"},
                {"id": "B", "label": "Hot"},
                {"id": "C", "label": "Exchange"},
            ],
            "rows": rows,
            "intra_pairs": pairs,
            "cross_asset_pairs": [],
        }))
        .expect("a request")
    }

    fn run_pass(request: &LedgerRequest) -> EngineResult<Normalized> {
        let book = Book::new(request).expect("a book");
        let [part] = book.assets.as_slice() else {
            panic!("one asset");
        };
        normalize(&book, part, &part.rows, &HashMap::new())
    }

    /// Row ids without their prefix.
    fn name(rows: &[Row], index: usize) -> &str {
        rows[index].id.trim_start_matches("custody-tax:")
    }

    fn quarantines(request: &LedgerRequest, normalized: &Normalized) -> Vec<Value> {
        normalized
            .quarantines
            .iter()
            .map(|item| {
                json!([
                    name(&request.rows, item.row),
                    item.reason.as_str(),
                    item.detail_kind.as_str(),
                    Value::Object(item.detail.clone()),
                ])
            })
            .collect()
    }

    fn items(request: &LedgerRequest, normalized: &Normalized) -> Vec<String> {
        normalized
            .ordered_items
            .iter()
            .map(|item| match *item {
                Item::Event(row) => format!("event {}", name(&request.rows, row)),
                Item::Transfer(row) => format!("transfer {}", name(&request.rows, row)),
            })
            .collect()
    }

    #[test]
    fn a_held_row_is_quarantined_and_unpairs_its_move() {
        let request = request(
            vec![
                row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                row(
                    "move-out",
                    "A",
                    "outbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "move", "fee_msat": 1_000_000, "hold": "conflicting_spend"}),
                ),
                row(
                    "move-in",
                    "B",
                    "inbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "move"}),
                ),
                row(
                    "late",
                    "B",
                    "inbound",
                    MAR,
                    10_000_000_000,
                    json!({"hold": "pending_onchain_confirmation"}),
                ),
            ],
            vec![pair("move-out", "move-in", Some("g"))],
            false,
        );
        let normalized = run_pass(&request).unwrap();
        assert_eq!(
            quarantines(&request, &normalized),
            vec![
                json!(["move-out", "conflicting_spend", "hold", {"wallet": "Cold", "asset": "BTC", "direction": "outbound"}]),
                json!(["late", "pending_onchain_confirmation", "hold", {
                    "wallet": "Hot", "asset": "BTC", "direction": "inbound", "required_for": "confirmed_chain_history",
                }]),
            ]
        );
        // The held leg's partner books on its own.
        assert_eq!(items(&request, &normalized), ["event buy", "event move-in"]);
        assert!(normalized.transfers.is_empty());
        assert_eq!(
            normalized.earliest_lot_contamination_at.as_deref(),
            Some(FEB)
        );
    }

    #[test]
    fn a_coarse_fee_bearing_move_is_reviewed_with_its_partner_leg_and_blocks_its_group() {
        let rows = vec![
            row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
            row(
                "coarse-out",
                "A",
                "outbound",
                FEB,
                10_000_000_000,
                json!({"anchor": "coarse", "fee_msat": 1_000_000, "coarse": true}),
            ),
            row(
                "coarse-in",
                "B",
                "inbound",
                FEB,
                10_000_000_000,
                json!({"anchor": "coarse"}),
            ),
            row(
                "sibling-out",
                "A",
                "outbound",
                FEB,
                10_000_000_000,
                json!({"anchor": "sibling", "fee_msat": 1_000_000}),
            ),
            row(
                "sibling-in",
                "B",
                "inbound",
                FEB,
                10_000_000_000,
                json!({"anchor": "sibling"}),
            ),
        ];
        let pairs = vec![
            pair("coarse-out", "coarse-in", Some("g")),
            pair("sibling-out", "sibling-in", Some("g")),
        ];
        let reviewed = request(rows.clone(), pairs.clone(), true);
        let normalized = run_pass(&reviewed).unwrap();
        assert_eq!(
            quarantines(&reviewed, &normalized),
            vec![
                json!(["coarse-out", "pricing_review_required", "pricing_review", {
                    "wallet": "Cold", "asset": "BTC", "direction": "transfer", "required_for": "pricing_review",
                    "transfer_group_id": "g",
                }]),
                json!(["coarse-in", "pricing_review_required", "partner_leg", {
                    "from_wallet": "Cold", "to_wallet": "Hot", "asset": "BTC", "direction": "transfer",
                    "paired_leg": true, "transfer_group_id": "g",
                }]),
                // One flag per transaction the group touches that nothing
                // names yet.
                json!(["sibling-out", "derived_transfer_group_blocked", "plain", {
                    "asset": "BTC", "wallet": "Cold", "direction": "transfer", "transfer_group_id": "g",
                    "blocked_by_reason": "pricing_review_required",
                }]),
            ]
        );
        assert!(normalized.transfers.is_empty());
        assert_eq!(items(&reviewed, &normalized), ["event buy"]);
        assert_eq!(
            normalized.earliest_lot_contamination_at.as_deref(),
            Some(FEB)
        );

        // Without the profile's opt-in, both moves book.
        let accepted = request(rows, pairs, false);
        let normalized = run_pass(&accepted).unwrap();
        assert!(normalized.quarantines.is_empty());
        assert_eq!(
            items(&accepted, &normalized),
            ["event buy", "transfer coarse-out", "transfer sibling-out"]
        );
        let moved: Vec<_> = normalized
            .transfers
            .iter()
            .map(|transfer| {
                (
                    transfer.pair,
                    transfer.sent.to_canonical(),
                    transfer.received.to_canonical(),
                    transfer.fee.to_canonical(),
                    transfer.group_id.clone(),
                )
            })
            .collect();
        let g = Some(String::from("g"));
        assert_eq!(
            moved,
            [
                (
                    0,
                    "10001E-5".into(),
                    "1E-1".into(),
                    "1E-5".into(),
                    g.clone()
                ),
                (1, "10001E-5".into(), "1E-1".into(), "1E-5".into(), g),
            ]
        );
        assert_eq!(normalized.earliest_lot_contamination_at, None);
    }

    #[test]
    fn a_move_without_an_out_price_is_priced_and_reviewed_on_its_in_leg() {
        let rows = |in_rate: Value| {
            vec![
                row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                row(
                    "m-out",
                    "A",
                    "outbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "m", "fee_msat": 1_000_000, "rate": null}),
                ),
                row(
                    "m-in",
                    "B",
                    "inbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "m", "rate": in_rate, "coarse": true}),
                ),
            ]
        };
        let ungrouped = || vec![pair("m-out", "m-in", None)];
        let reviewed = request(rows(json!("41000E0")), ungrouped(), true);
        let normalized = run_pass(&reviewed).unwrap();
        assert_eq!(
            quarantines(&reviewed, &normalized),
            vec![
                json!(["m-in", "pricing_review_required", "pricing_review", {
                    "wallet": "Hot", "asset": "BTC", "direction": "transfer", "required_for": "pricing_review",
                }]),
                json!(["m-out", "pricing_review_required", "partner_leg", {
                    "from_wallet": "Cold", "to_wallet": "Hot", "asset": "BTC", "direction": "transfer", "paired_leg": true,
                }]),
            ]
        );

        let accepted = request(rows(json!("41000E0")), ungrouped(), false);
        let normalized = run_pass(&accepted).unwrap();
        assert_eq!(
            normalized.transfers[0].spot_price,
            Some(Fiat::Finite(dec("41000E0")))
        );

        // Neither leg priced: the move's fee cannot be valued.
        let unpriced = request(rows(Value::Null), ungrouped(), true);
        let normalized = run_pass(&unpriced).unwrap();
        assert_eq!(
            quarantines(&unpriced, &normalized),
            vec![
                json!(["m-out", "missing_spot_price", "plain", {
                    "from_wallet": "Cold", "to_wallet": "Hot", "asset": "BTC", "direction": "transfer",
                    "required_for": "transfer_fee",
                }]),
                json!(["m-in", "missing_spot_price", "partner_leg", {
                    "from_wallet": "Cold", "to_wallet": "Hot", "asset": "BTC", "direction": "transfer", "paired_leg": true,
                }]),
            ]
        );
    }

    #[test]
    fn coarse_events_are_reviewed_only_when_the_profile_asks() {
        let rows = vec![
            row(
                "coarse-buy",
                "A",
                "inbound",
                JAN,
                100_000_000_000,
                json!({"coarse": true}),
            ),
            row(
                "coarse-sell",
                "A",
                "outbound",
                FEB,
                10_000_000_000,
                json!({"coarse": true, "fee_msat": 1_000}),
            ),
            row("buy", "A", "inbound", MAR, 10_000_000_000, json!({})),
        ];
        let reviewed = request(rows.clone(), Vec::new(), true);
        let normalized = run_pass(&reviewed).unwrap();
        let review = |direction: &str| json!({"wallet": "Cold", "asset": "BTC", "direction": direction, "required_for": "pricing_review"});
        assert_eq!(
            quarantines(&reviewed, &normalized),
            vec![
                json!([
                    "coarse-buy",
                    "pricing_review_required",
                    "pricing_review",
                    review("inbound")
                ]),
                json!([
                    "coarse-sell",
                    "pricing_review_required",
                    "pricing_review",
                    review("outbound")
                ]),
            ]
        );
        assert_eq!(items(&reviewed, &normalized), ["event buy"]);

        let accepted = request(rows, Vec::new(), false);
        let normalized = run_pass(&accepted).unwrap();
        assert!(normalized.quarantines.is_empty());
        assert_eq!(
            items(&accepted, &normalized),
            ["event coarse-buy", "event coarse-sell", "event buy"]
        );
    }

    #[test]
    fn contamination_starts_at_the_earliest_slice_of_a_quarantined_transaction() {
        let request = request(
            vec![
                row(
                    "early-slice",
                    "A",
                    "inbound",
                    JAN,
                    50_000_000_000,
                    json!({"anchor": "tx"}),
                ),
                row(
                    "late-slice",
                    "B",
                    "inbound",
                    MAR,
                    50_000_000_000,
                    json!({"anchor": "tx", "rate": null}),
                ),
                row(
                    "unpriced",
                    "A",
                    "inbound",
                    APR,
                    10_000_000_000,
                    json!({"rate": null}),
                ),
                row(
                    "untimed",
                    "A",
                    "inbound",
                    "",
                    10_000_000_000,
                    json!({"rate": null}),
                ),
            ],
            Vec::new(),
            false,
        );
        let normalized = run_pass(&request).unwrap();
        let reasons: Vec<_> = normalized
            .quarantines
            .iter()
            .map(|item| (name(&request.rows, item.row), item.reason.as_str()))
            .collect();
        assert_eq!(
            reasons,
            [
                ("late-slice", "missing_spot_price"),
                ("unpriced", "missing_spot_price"),
                ("untimed", "missing_spot_price"),
            ]
        );
        // The priced slice of the quarantined transaction taints from its
        // own time; a row without a time taints nothing.
        assert_eq!(
            normalized.earliest_lot_contamination_at.as_deref(),
            Some(JAN)
        );
    }

    #[test]
    fn a_blocked_group_flags_each_transaction_once() {
        let request = request(
            vec![
                row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                row("buy-b", "B", "inbound", JAN, 100_000_000_000, json!({})),
                row(
                    "g1-fail-out",
                    "A",
                    "outbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "g1-fail", "fee_msat": 1_000_000, "rate": null}),
                ),
                row(
                    "g1-fail-in",
                    "B",
                    "inbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "g1-fail", "rate": null}),
                ),
                row(
                    "g1-pass-out",
                    "A",
                    "outbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "g1-pass", "fee_msat": 1_000_000}),
                ),
                row(
                    "g1-pass-in",
                    "C",
                    "inbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "g1-pass"}),
                ),
                row(
                    "g2-fail-out",
                    "B",
                    "outbound",
                    MAR,
                    10_000_000_000,
                    json!({"anchor": "g2-fail", "fee_msat": 1_000_000, "rate": null}),
                ),
                row(
                    "g2-fail-in",
                    "C",
                    "inbound",
                    MAR,
                    10_000_000_000,
                    json!({"anchor": "g2-fail", "rate": null}),
                ),
                // Another slice of the transaction the first group's block
                // flags, moved in the second group.
                row(
                    "g2-out",
                    "A",
                    "outbound",
                    MAR,
                    20_000_000_000,
                    json!({"anchor": "g1-pass"}),
                ),
                row(
                    "g2-in",
                    "C",
                    "inbound",
                    MAR,
                    20_000_000_000,
                    json!({"anchor": "g2-new"}),
                ),
            ],
            vec![
                pair("g1-fail-out", "g1-fail-in", Some("g1")),
                pair("g1-pass-out", "g1-pass-in", Some("g1")),
                pair("g2-fail-out", "g2-fail-in", Some("g2")),
                pair("g2-out", "g2-in", Some("g2")),
            ],
            false,
        );
        let normalized = run_pass(&request).unwrap();
        let reasons: Vec<_> = normalized
            .quarantines
            .iter()
            .map(|item| {
                (
                    name(&request.rows, item.row),
                    item.reason.as_str(),
                    item.detail.get("transfer_group_id").cloned(),
                )
            })
            .collect();
        let group = |id: &str| Some(Value::from(id));
        assert_eq!(
            reasons,
            [
                ("g1-fail-out", "missing_spot_price", group("g1")),
                ("g1-fail-in", "missing_spot_price", group("g1")),
                ("g2-fail-out", "missing_spot_price", group("g2")),
                ("g2-fail-in", "missing_spot_price", group("g2")),
                ("g1-pass-out", "derived_transfer_group_blocked", group("g1")),
                ("g2-in", "derived_transfer_group_blocked", group("g2")),
            ]
        );
        assert!(normalized.transfers.is_empty());
        assert_eq!(items(&request, &normalized), ["event buy", "event buy-b"]);
    }

    #[test]
    fn a_group_blocked_twice_keeps_its_first_reason() {
        let request = request(
            vec![
                row("buy", "A", "inbound", JAN, 100_000_000_000, json!({})),
                row(
                    "coarse-out",
                    "A",
                    "outbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "coarse", "fee_msat": 1_000_000, "coarse": true}),
                ),
                row(
                    "coarse-in",
                    "B",
                    "inbound",
                    FEB,
                    10_000_000_000,
                    json!({"anchor": "coarse"}),
                ),
                row(
                    "unpriced-out",
                    "A",
                    "outbound",
                    MAR,
                    10_000_000_000,
                    json!({"anchor": "unpriced", "fee_msat": 1_000_000, "rate": null}),
                ),
                row(
                    "unpriced-in",
                    "B",
                    "inbound",
                    MAR,
                    10_000_000_000,
                    json!({"anchor": "unpriced", "rate": null}),
                ),
                row(
                    "sibling-out",
                    "A",
                    "outbound",
                    APR,
                    10_000_000_000,
                    json!({"anchor": "sibling", "fee_msat": 1_000_000}),
                ),
                row(
                    "sibling-in",
                    "C",
                    "inbound",
                    APR,
                    10_000_000_000,
                    json!({"anchor": "sibling"}),
                ),
            ],
            vec![
                pair("coarse-out", "coarse-in", Some("g")),
                pair("unpriced-out", "unpriced-in", Some("g")),
                pair("sibling-out", "sibling-in", Some("g")),
            ],
            true,
        );
        let normalized = run_pass(&request).unwrap();
        let reasons: Vec<_> = normalized
            .quarantines
            .iter()
            .map(|item| {
                (
                    name(&request.rows, item.row),
                    item.reason.as_str(),
                    item.detail.get("blocked_by_reason").cloned(),
                )
            })
            .collect();
        // The sibling is blocked by the group's first reason, not its last.
        let blocked_by = Some(Value::from("pricing_review_required"));
        assert_eq!(
            reasons,
            [
                ("coarse-out", "pricing_review_required", None),
                ("coarse-in", "pricing_review_required", None),
                ("unpriced-out", "missing_spot_price", None),
                ("unpriced-in", "missing_spot_price", None),
                ("sibling-out", "derived_transfer_group_blocked", blocked_by),
            ]
        );
        assert!(normalized.transfers.is_empty());
        assert_eq!(items(&request, &normalized), ["event buy"]);
    }

    /// Each event's regime and basis, and each move's regime and flows, by
    /// row name.
    fn regimes(request: &LedgerRequest, normalized: &Normalized) -> Vec<Value> {
        let rows = &request.rows;
        normalized
            .events
            .iter()
            .map(|event| {
                json!([
                    name(rows, event.row),
                    event.at_regime.map(Regime::as_str),
                    event.at_regime_basis,
                ])
            })
            .chain(normalized.transfers.iter().map(|transfer| {
                let flows = transfer.regime_flows.as_ref().map(|flows| {
                    [
                        flows.out.alt,
                        flows.out.neu,
                        flows.inbound.alt,
                        flows.inbound.neu,
                    ]
                });
                json!([
                    name(rows, transfer.out),
                    transfer.at_regime.map(Regime::as_str),
                    flows
                ])
            }))
            .collect()
    }

    #[test]
    fn austrian_events_and_moves_take_the_trackers_regimes_and_their_overrides() {
        let request = austrian(
            vec![
                row("alt-buy", "A", "inbound", ALT, 100_000_000_000, json!({})),
                // An inbound override is the event's regime, but the tracker
                // keeps the time's: B holds Alt only.
                row(
                    "said-neu",
                    "B",
                    "inbound",
                    ALT,
                    10_000_000_000,
                    json!({"regime_override": "neu"}),
                ),
                row("neu-buy", "A", "inbound", JAN, 50_000_000_000, json!({})),
                // A holds both regimes: an election.
                row("sale", "A", "outbound", FEB, 1_000_000_000, json!({})),
                // An override drops the election.
                row(
                    "said-alt",
                    "A",
                    "outbound",
                    FEB,
                    1_000_000_000,
                    json!({"regime_override": "alt"}),
                ),
                row("b-sale", "B", "outbound", FEB, 1_000_000_000, json!({})),
                row(
                    "move-out",
                    "A",
                    "outbound",
                    MAR,
                    60_000_000_000,
                    json!({"anchor": "move", "fee_msat": 1_000_000}),
                ),
                row(
                    "move-in",
                    "C",
                    "inbound",
                    MAR,
                    60_000_000_000,
                    json!({"anchor": "move"}),
                ),
                row(
                    "over-out",
                    "C",
                    "outbound",
                    APR,
                    5_000_000_000,
                    json!({"anchor": "over", "regime_override": "alt"}),
                ),
                row(
                    "over-in",
                    "B",
                    "inbound",
                    APR,
                    5_000_000_000,
                    json!({"anchor": "over"}),
                ),
            ],
            vec![
                pair("move-out", "move-in", None),
                pair("over-out", "over-in", None),
            ],
        );
        let normalized = run_pass(&request).unwrap();
        // The move spends A's Neu left after the election (48 BTC) first,
        // its fee from that slice, then 12.001 BTC of Alt; its override
        // twin keeps its own flows.
        assert_eq!(
            regimes(&request, &normalized),
            vec![
                json!(["alt-buy", "alt", null]),
                json!(["said-neu", "neu", null]),
                json!(["neu-buy", "neu", null]),
                json!(["sale", "neu", "wahlrecht"]),
                json!(["said-alt", "alt", null]),
                json!(["b-sale", "alt", null]),
                json!([
                    "move-out",
                    "neu",
                    [
                        12_001_000_000_u64,
                        48_000_000_000_u64,
                        12_001_000_000_u64,
                        47_999_000_000_u64
                    ]
                ]),
                json!([
                    "over-out",
                    "alt",
                    [0, 5_000_000_000_u64, 0, 5_000_000_000_u64]
                ]),
            ]
        );
        // A generic book has none of these.
        let mut generic = request.clone();
        generic.policy.country = crate::ledger::request::Country::Generic {
            long_term_days: 365,
        };
        let normalized = run_pass(&generic).unwrap();
        assert!(normalized
            .events
            .iter()
            .all(|event| event.at_regime.is_none() && event.at_regime_basis.is_none()));
        assert!(normalized
            .transfers
            .iter()
            .all(|transfer| transfer.at_regime.is_none() && transfer.regime_flows.is_none()));
    }

    #[test]
    fn an_austrian_time_the_tracker_cannot_read_fails_before_normalize_reads_a_price() {
        // The tracker reads every row first, so its error comes before the
        // NaN rate's of a row it visits earlier.
        let request = austrian(
            vec![
                row("nan", "A", "inbound", JAN, 1_000, json!({"rate": "NaN"})),
                row("unread", "A", "inbound", "a time", 1_000, json!({})),
            ],
            Vec::new(),
        );
        let error = run_pass(&request).unwrap_err();
        assert_eq!(error.class, ErrorClass::BuiltinValueError);
        assert_eq!(error.message, "Invalid isoformat string: 'a time'");
        // A held row is not the tracker's: its time is never read.
        let mut held = request.clone();
        held.rows[1].hold = Some(Hold::PendingOnchainConfirmation);
        let error = run_pass(&held).unwrap_err();
        assert_eq!(error.class, ErrorClass::InvalidOperation);
    }
}
