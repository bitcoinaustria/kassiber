//! Composition (design §4.8): one computed asset's part of the journal
//! ledger, as the adapter composes it from the lot engine's results
//! (`_append_rp2_journal_entries`, `_accumulate_asset_holdings`, and
//! `_build_tax_summary_rows`, `tests/tax_oracle/adapter.py:1388-1666`; the
//! fragment values of `kassiber/core/engines/explanation.py`):
//!
//! - an acquisition per visible lot, in lot order, at its effective basis,
//!   with its unit cost;
//! - the realized entries: the gain/loss fragments grouped by their taxable
//!   event and holding period, in the order each group first appears, each
//!   the sum of its fragments from `Decimal("0")`. An earn receipt is
//!   income, a move whose fee is taxable is a transfer fee, a fee-only spend
//!   is a fee, and everything else a disposal. A fee or transfer fee
//!   realizes no gain (its proceeds are its basis), and every realized
//!   quantity but income's is negated. On an Austrian book every fragment,
//!   fees and income included, has the lot engine's category, which the
//!   row's overrides may replace or clear (`adapter.py:1453-1471`); the
//!   first fragment whose classification failed fails the composition.
//!   Fragments group by their category too, so one event can give several
//!   entries, and a fee or transfer fee drops its category only after that
//!   split (A map §9.9);
//! - the two legs of each move the walk booked, in audit order;
//! - holdings: each positive balance of a wallet with a label, valued at
//!   the open lots' average basis;
//! - the tax summary: the yearly totals without fees and moves, by year,
//!   type, and holding period.
//!
//! Every sum and product is CPython's, one 32-digit step each, and every
//! comparison exact, as the adapter computes them on plain `Decimal`s. The
//! reader (`kassiber/core/engines/ledger_response.py`) adds what is not tax
//! logic: ids, wallet and account references, times, floats, and each
//! fragment's row facts, and an Austrian category's Kennzahl.

use std::collections::{HashMap, HashSet};

use crate::decimal::Decimal;
use crate::entry::{Detail, Entry, TransactionType};
use crate::error::{EngineError, EngineResult};
use crate::manifest::{EntryType, NON_REPORTABLE_CATEGORY_OVERRIDES};
use crate::model::{AssetOutput, Dec, EntryKind, GainLossOut};
use crate::num;

use super::request::{Country, Row};
use super::response::{
    AcquisitionOut, AuditOut, EntryOut, FragmentEvent, FragmentLot, FragmentOut, HoldingOut,
    RealizedOut, SummaryOut, TransferLegOut,
};
use super::run::Book;
use super::walk::BuiltEntry;

/// One asset's journal entries, holdings, and tax-summary rows.
#[derive(Clone, Debug, Default)]
pub(crate) struct Composed {
    pub entries: Vec<EntryOut>,
    pub holdings: Vec<HoldingOut>,
    pub tax_summary: Vec<SummaryOut>,
}

/// One computed asset: the walk's entries, the lot engine's results, and
/// the asset's audit rows, which start at `first_audit` in the response.
pub(crate) struct ComputedAsset<'c> {
    pub entries: &'c [BuiltEntry],
    pub output: &'c AssetOutput,
    pub audit: &'c [AuditOut],
    pub first_audit: usize,
}

/// Composes one computed asset, failing where the adapter's composition
/// does: its entries, then its holdings.
pub(crate) fn compose(book: &Book, asset: &ComputedAsset) -> EngineResult<Composed> {
    let composer = Composer::new(book, asset);
    let mut entries = composer.acquisitions()?;
    entries.extend(composer.realized()?);
    entries.extend(composer.transfer_legs());
    let holdings = composer.holdings()?;
    Ok(Composed {
        entries,
        holdings,
        tax_summary: composer.tax_summary(),
    })
}

fn internal(message: &str) -> EngineError {
    EngineError::runtime(format!("Internal error: {message}"))
}

/// The wallet label an entry's journal rows are booked to (`_wallet_for`):
/// the account an acquisition or disposal names, a move's source.
fn wallet_label(entry: &Entry) -> &str {
    match &entry.detail {
        Detail::In { exchange, .. } | Detail::Out { exchange, .. } => exchange,
        Detail::Intra { from_exchange, .. } => from_exchange,
    }
}

/// An entry's description: its notes. The walk composes notes for every
/// entry, and they are never empty (an event's text falls back to its row
/// id, a move's to `Transfer <from> -> <to>`), so the adapter's fallback to
/// the row's description is never taken.
fn description(entry: &Entry) -> EngineResult<String> {
    if entry.notes.is_empty() {
        return Err(internal("an entry without notes"));
    }
    Ok(entry.notes.clone())
}

/// The category an Austrian fragment reports after its row's overrides
/// (`adapter.py:1458-1470`): with taxability off, a non-reportable override
/// is kept and anything else clears the category; the override `none`
/// clears it too; any other override replaces it.
fn overridden(row: &Row, category: String) -> Option<String> {
    let chosen = row.category_override.as_deref();
    match chosen {
        Some(chosen)
            if row.taxability_off && NON_REPORTABLE_CATEGORY_OVERRIDES.contains(&chosen) =>
        {
            Some(chosen.to_owned())
        }
        _ if row.taxability_off => None,
        Some("none") => None,
        Some(chosen) => Some(chosen.to_owned()),
        None => Some(category),
    }
}

/// One realized entry while its fragments are summed.
struct Realized {
    entry_type: EntryType,
    row: String,
    wallet: String,
    asset: String,
    description: String,
    long_term: bool,
    /// The Austrian category after the overrides.
    at_category: Option<String>,
    quantity: Decimal,
    cost_basis: Decimal,
    proceeds: Decimal,
    gain_loss: Decimal,
    fragments: Vec<FragmentOut>,
}

struct Composer<'c, 'a> {
    book: &'c Book<'a>,
    asset: &'c ComputedAsset<'c>,
    /// The walk's entries by row number (unique within an asset).
    by_number: HashMap<i64, &'c BuiltEntry>,
    /// Each visible lot's effective basis, by row number.
    basis: HashMap<i64, &'c Decimal>,
}

impl<'c, 'a> Composer<'c, 'a> {
    fn new(book: &'c Book<'a>, asset: &'c ComputedAsset<'c>) -> Self {
        let by_number = asset
            .entries
            .iter()
            .map(|built| (built.entry.row, built))
            .collect();
        let output = asset.output;
        let basis = output
            .in_transactions
            .iter()
            .zip(&output.in_fiat_in_with_fee)
            .map(|(row, basis)| (*row, &basis.0))
            .collect();
        Composer {
            book,
            asset,
            by_number,
            basis,
        }
    }

    fn entry(&self, number: i64, kind: EntryKind) -> EngineResult<&'c BuiltEntry> {
        match self.by_number.get(&number) {
            Some(built) if built.entry.kind == kind => Ok(built),
            _ => Err(internal("the lot engine returned an unknown entry")),
        }
    }

    fn row_id(&self, built: &BuiltEntry) -> String {
        self.book.rows[built.row].id.clone()
    }

    /// A lot's effective basis (`get_in_transaction_fiat_in_with_fee`): the
    /// lot engine's, else the lot's own.
    fn lot_basis(&self, lot: &'c Entry) -> EngineResult<&'c Decimal> {
        match self.basis.get(&lot.row) {
            Some(basis) => Ok(basis),
            None => lot
                .fiat_in_with_fee()
                .ok_or_else(|| internal("a lot is not an acquisition")),
        }
    }

    /// An Austrian fragment's category (`_classify_at_disposal`): the lot
    /// engine's, or the failure it recorded for it, raised here.
    fn classify(gain_loss: &GainLossOut) -> EngineResult<String> {
        if let Some(error) = &gain_loss.at_category_error {
            return Err(EngineError::new(error.class, error.message.clone()));
        }
        gain_loss.at_category.clone().ok_or_else(|| {
            EngineError::runtime(
                "Internal error: the engine returned no Austrian category for this gain/loss",
            )
        })
    }

    fn crypto_in(lot: &Entry) -> EngineResult<&Decimal> {
        lot.crypto_in()
            .ok_or_else(|| internal("a lot is not an acquisition"))
    }

    /// An acquisition per visible lot, in lot order.
    fn acquisitions(&self) -> EngineResult<Vec<EntryOut>> {
        let output = self.asset.output;
        let mut entries = Vec::with_capacity(output.in_transactions.len());
        for (number, basis) in output
            .in_transactions
            .iter()
            .zip(&output.in_fiat_in_with_fee)
        {
            let built = self.entry(*number, EntryKind::In)?;
            let lot = &built.entry;
            let quantity = Self::crypto_in(lot)?;
            let unit_cost = num::div(&basis.0, quantity)?;
            entries.push(EntryOut::Acquisition(AcquisitionOut {
                entry_type: EntryType::Acquisition,
                row: self.row_id(built),
                wallet: wallet_label(lot).to_owned(),
                asset: lot.asset.clone(),
                quantity: Dec(quantity.clone()),
                fiat_value: basis.clone(),
                unit_cost: Dec(unit_cost),
                description: description(lot)?,
            }));
        }
        Ok(entries)
    }

    /// The fragment's values and the facts of its event and lot.
    fn fragment(
        &self,
        gain_loss: &GainLossOut,
        event: &'c BuiltEntry,
    ) -> EngineResult<FragmentOut> {
        let derived = event.entry.derived();
        let lot = match gain_loss.lot {
            None => None,
            Some(number) => {
                let built = self.entry(number, EntryKind::In)?;
                let lot = built.entry.derived();
                Some(FragmentLot {
                    row: self.row_id(built),
                    spot_price: lot.spot_price,
                    fiat_fee: lot.fiat_fee,
                    crypto_fee: lot.crypto_fee,
                    basis: Dec(self.lot_basis(&built.entry)?.clone()),
                    crypto_in: Dec(Self::crypto_in(&built.entry)?.clone()),
                })
            }
        };
        Ok(FragmentOut {
            crypto_amount: gain_loss.crypto_amount.clone(),
            cost_basis: gain_loss.fiat_cost_basis.clone(),
            proceeds: gain_loss.proceeds.clone(),
            gain_loss: gain_loss.fiat_gain.clone(),
            unit_basis_override: gain_loss.unit_cost_basis_override.clone(),
            event: FragmentEvent {
                row: self.row_id(event),
                spot_price: derived.spot_price,
                fiat_fee: derived.fiat_fee,
                crypto_fee: derived.crypto_fee,
            },
            lot,
        })
    }

    /// The realized entries, in the order their groups first appear.
    fn realized(&self) -> EngineResult<Vec<EntryOut>> {
        // The moves the walk booked, by their out leg (`audit_by_rp2_id`).
        let audited: HashSet<&str> = self
            .asset
            .audit
            .iter()
            .map(|audit| self.book.request.intra_pairs[audit.pair].out.as_str())
            .collect();
        let austrian = matches!(self.book.request.policy.country, Country::At {});
        // By taxable event (row number), holding period, and on an Austrian
        // book category, in the order they first appear.
        let mut groups: Vec<Realized> = Vec::new();
        let mut positions: HashMap<(i64, bool, Option<String>), usize> = HashMap::new();
        for gain_loss in &self.asset.output.gain_losses {
            let event = self.entry(gain_loss.event.row, gain_loss.event.kind)?;
            let entry = &event.entry;
            let kind = entry.transaction_type;
            let booked_move = audited.contains(entry.unique_id.as_str());
            let entry_type = if kind.is_earn() {
                EntryType::Income
            } else if booked_move && kind == TransactionType::Move {
                EntryType::TransferFee
            } else if kind == TransactionType::Fee {
                EntryType::Fee
            } else {
                EntryType::Disposal
            };
            let at_category = if austrian {
                let category = Self::classify(gain_loss)?;
                // The overrides of the event's row (an event's own, a
                // move's out leg).
                overridden(&self.book.rows[event.row], category)
            } else {
                None
            };
            let key = (entry.row, gain_loss.long_term, at_category.clone());
            let position = match positions.get(&key) {
                Some(&position) => position,
                None => {
                    positions.insert(key, groups.len());
                    groups.push(Realized {
                        entry_type,
                        row: self.row_id(event),
                        wallet: wallet_label(entry).to_owned(),
                        asset: entry.asset.clone(),
                        description: description(entry)?,
                        long_term: gain_loss.long_term,
                        at_category,
                        quantity: num::zero(),
                        cost_basis: num::zero(),
                        proceeds: num::zero(),
                        gain_loss: num::zero(),
                        fragments: Vec::new(),
                    });
                    groups.len() - 1
                }
            };
            let fragment = self.fragment(gain_loss, event)?;
            let group = &mut groups[position];
            group.fragments.push(fragment);
            group.quantity = num::add(&group.quantity, &gain_loss.crypto_amount.0)?;
            group.cost_basis = num::add(&group.cost_basis, &gain_loss.fiat_cost_basis.0)?;
            group.proceeds = num::add(&group.proceeds, &gain_loss.proceeds.0)?;
            group.gain_loss = num::add(&group.gain_loss, &gain_loss.fiat_gain.0)?;
        }
        groups
            .into_iter()
            .map(|group| {
                // Fees reduce the balance and its basis but realize nothing,
                // and report no category.
                let fee = matches!(group.entry_type, EntryType::Fee | EntryType::TransferFee);
                let (proceeds, gain_loss, at_category) = if fee {
                    (group.cost_basis.clone(), num::zero(), None)
                } else {
                    (group.proceeds, group.gain_loss, group.at_category)
                };
                let quantity = if group.entry_type == EntryType::Income {
                    group.quantity
                } else {
                    num::neg(&group.quantity)?
                };
                Ok(EntryOut::Realized(RealizedOut {
                    entry_type: group.entry_type,
                    row: group.row,
                    wallet: group.wallet,
                    asset: group.asset,
                    quantity: Dec(quantity),
                    proceeds: Dec(proceeds),
                    cost_basis: Dec(group.cost_basis),
                    gain_loss: Dec(gain_loss),
                    description: group.description,
                    at_category,
                    capital_gains_type: if group.long_term { "long" } else { "short" },
                    fragments: group.fragments,
                }))
            })
            .collect()
    }

    /// The out and in leg of each booked move, in audit order.
    fn transfer_legs(&self) -> Vec<EntryOut> {
        (0..self.asset.audit.len())
            .flat_map(|index| {
                let audit = self.asset.first_audit + index;
                [EntryType::TransferOut, EntryType::TransferIn]
                    .map(|entry_type| EntryOut::TransferLeg(TransferLegOut { entry_type, audit }))
            })
            .collect()
    }

    /// Each positive balance of a labelled wallet, at the open lots' average
    /// basis (`_accumulate_asset_holdings`).
    fn holdings(&self) -> EngineResult<Vec<HoldingOut>> {
        let output = self.asset.output;
        let one = Decimal::from(1u8);
        let mut total_quantity = num::zero();
        let mut total_cost_basis = num::zero();
        for position in &output.open_positions {
            let remaining = num::sub(&one, &position.sold_percentage.0)?;
            if remaining <= num::zero() {
                continue;
            }
            let lot = &self.entry(position.row, EntryKind::In)?.entry;
            let quantity = num::mul(Self::crypto_in(lot)?, &remaining)?;
            total_quantity = num::add(&total_quantity, &quantity)?;
            let cost_basis = num::mul(&position.fiat_in_with_fee.0, &remaining)?;
            total_cost_basis = num::add(&total_cost_basis, &cost_basis)?;
        }
        if total_quantity <= num::zero() {
            return Ok(Vec::new());
        }
        let average = num::div(&total_cost_basis, &total_quantity)?;
        // The reader books by label (`wallet_refs_by_label`): a balance of
        // a wallet without a label is skipped.
        let labels: HashSet<&str> = self
            .book
            .request
            .wallets
            .iter()
            .filter_map(|wallet| wallet.label.as_deref())
            .filter(|label| !label.is_empty())
            .collect();
        let mut holdings = Vec::new();
        for balance in &output.balances {
            if !labels.contains(balance.exchange.as_str()) {
                continue;
            }
            let quantity = &balance.final_balance.0;
            if *quantity <= num::zero() {
                continue;
            }
            holdings.push(HoldingOut {
                asset: output.asset.clone(),
                wallet: balance.exchange.clone(),
                quantity: Dec(quantity.clone()),
                cost_basis: Dec(num::mul(quantity, &average)?),
            });
        }
        Ok(holdings)
    }

    /// The yearly totals without fees and moves, by year, type, and holding
    /// period (`_build_tax_summary_rows`).
    fn tax_summary(&self) -> Vec<SummaryOut> {
        let output = self.asset.output;
        let mut rows: Vec<SummaryOut> = output
            .yearly
            .iter()
            .filter(|yearly| !matches!(yearly.transaction_type, "fee" | "move"))
            .map(|yearly| SummaryOut {
                year: yearly.year,
                asset: output.asset.clone(),
                transaction_type: yearly.transaction_type,
                long_term: yearly.long_term,
                quantity: yearly.crypto_amount.clone(),
                proceeds: yearly.fiat_amount.clone(),
                cost_basis: yearly.fiat_cost_basis.clone(),
                gain_loss: yearly.fiat_gain_loss.clone(),
            })
            .collect();
        rows.sort_by_key(|row| (row.year, row.transaction_type, row.long_term));
        rows
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::ledger::request::LedgerRequest;
    use crate::ledger::run::ledger;
    use crate::time::CivilDate;

    // Small books through the whole pipeline. Every expectation is
    // what the oracle gives for the same rows (a projection built with
    // `tests/tax_ledger_generator.py`'s `projected` and `direct`, compared
    // with `tests/tax_engine_compare.py`: production equals the oracle on
    // each); the row texts here are the row ids instead.

    /// A parsed timestamp at midnight UTC on `date` (`YYYY-MM-DD`).
    fn ts(date: &str) -> Value {
        let days = CivilDate::parse(date).expect("a date").to_days();
        let year: i32 = date[..4].parse().expect("a year");
        json!({
            "raw": format!("{date}T00:00:00Z"),
            "us": days * 86_400 * 1_000_000,
            "offset_s": 0,
            "date": date,
            "year": year,
            "vienna_date": date,
            "display": format!("{date} 00:00:00+00:00"),
            "iso": format!("{date}T00:00:00+00:00"),
        })
    }

    /// A BTC row priced at 40000, whose id is its transaction (anchor)
    /// unless `fields` says otherwise.
    fn row(
        id: &str,
        wallet: &str,
        direction: &str,
        date: &str,
        amount: u64,
        fields: Value,
    ) -> Value {
        let mut row = json!({
            "id": format!("custody-tax:{id}"),
            "anchor": id,
            "asset": "BTC",
            "wallet_id": wallet,
            "direction": direction,
            "ts": ts(date),
            "cutoff_instant": {"us": ts(date)["us"]},
            "amount_msat": amount,
            "fee_msat": 0,
            "rate": "40000E0",
            "value": null,
            "coarse": false,
            "kind": if direction == "inbound" { "buy" } else { "sell" },
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

    /// The two legs of a move of `amount` from `from` to `to`.
    fn legs(id: &str, from: &str, to: &str, date: &str, amount: u64, fields: Value) -> [Value; 2] {
        let mut out = row(
            &format!("{id}-out"),
            from,
            "outbound",
            date,
            amount,
            fields.clone(),
        );
        let mut inbound = row(&format!("{id}-in"), to, "inbound", date, amount, fields);
        for leg in [&mut out, &mut inbound] {
            leg["anchor"] = json!(id);
            leg["kind"] = json!("transfer");
        }
        inbound["fee_msat"] = json!(0);
        [out, inbound]
    }

    fn pair(id: &str) -> Value {
        json!({
            "out": format!("custody-tax:{id}-out"),
            "in": format!("custody-tax:{id}-in"),
            "source": "reviewed_custody_component",
            "group_id": null,
        })
    }

    fn request(method: Value, label: &str, rows: Vec<Value>, pairs: Vec<Value>) -> LedgerRequest {
        book_request(
            json!({"kind": "generic", "long_term_days": 365}),
            method,
            label,
            rows,
            pairs,
        )
    }

    fn austrian(method: &str, rows: Vec<Value>, pairs: Vec<Value>) -> LedgerRequest {
        book_request(json!({"kind": "at"}), json!(method), method, rows, pairs)
    }

    fn book_request(
        country: Value,
        method: Value,
        label: &str,
        rows: Vec<Value>,
        pairs: Vec<Value>,
    ) -> LedgerRequest {
        serde_json::from_value(json!({
            "schema_version": 2,
            "operation": "ledger",
            "policy": {
                "country": country,
                "method": method,
                "method_label": label,
                "require_coarse_review": false,
            },
            "config": {
                "assets": ["BTC", "LBTC"],
                "exchanges": ["Cold", "Exchange", "Hot", "Liquid"],
                "holders": ["Default"],
            },
            "holder": "Default",
            "wallets": [
                {"id": "A", "label": "Cold"},
                {"id": "B", "label": "Hot"},
                {"id": "C", "label": "Exchange"},
                {"id": "L", "label": "Liquid"},
            ],
            "rows": rows,
            "intra_pairs": pairs,
            "cross_asset_pairs": [],
        }))
        .expect("a request")
    }

    /// The ledger response as JSON, or the failure as its document.
    fn run(request: &LedgerRequest) -> Value {
        match ledger(request) {
            Ok(response) => serde_json::to_value(response).expect("serializes"),
            Err(error) => json!({"error": serde_json::to_value(error).expect("serializes")}),
        }
    }

    /// A FIFO book over two years: a long-held lot, a purchase at a value,
    /// staking income, a sale across both holding periods, and a fee-only
    /// spend.
    fn fifo_book() -> Value {
        run(&request(
            json!("fifo"),
            "FIFO",
            vec![
                row(
                    "old",
                    "A",
                    "inbound",
                    "2022-01-01",
                    100_000_000_000,
                    json!({"rate": "30000E0"}),
                ),
                row(
                    "sale23",
                    "A",
                    "outbound",
                    "2023-06-01",
                    20_000_000_000,
                    json!({"rate": "35000E0"}),
                ),
                row(
                    "new",
                    "A",
                    "inbound",
                    "2024-01-01",
                    300_000_000_000,
                    json!({"rate": null, "value": "100000E0"}),
                ),
                row(
                    "earn",
                    "A",
                    "inbound",
                    "2024-02-01",
                    10_000_000_000,
                    json!({"kind": "staking"}),
                ),
                row(
                    "sell",
                    "A",
                    "outbound",
                    "2024-03-01",
                    150_000_000_000,
                    json!({"fee_msat": 100_000_000, "rate": "50000E0"}),
                ),
                row(
                    "feeonly",
                    "A",
                    "outbound",
                    "2024-03-02",
                    0,
                    json!({"fee_msat": 10_000_000, "rate": "50000E0", "kind": "withdrawal"}),
                ),
            ],
            vec![],
        ))
    }

    /// Two BTC moves (one with a fee, one without) and an LBTC move with a
    /// fee, each asset bought first.
    fn moves_book() -> Value {
        let [m1_out, m1_in] = legs("m1", "A", "B", "2024-02-01", 50_000_000_000, json!({}));
        let [m2_out, m2_in] = legs("m2", "B", "C", "2024-03-01", 20_000_000_000, json!({}));
        let [lm_out, lm_in] = legs(
            "lm",
            "L",
            "C",
            "2024-04-01",
            5_000_000_000,
            json!({"asset": "LBTC"}),
        );
        let mut m1_out = m1_out;
        m1_out["fee_msat"] = json!(10_000_000);
        let mut lm_out = lm_out;
        lm_out["fee_msat"] = json!(1_000_000);
        run(&request(
            json!("fifo"),
            "FIFO",
            vec![
                row(
                    "buy",
                    "A",
                    "inbound",
                    "2024-01-01",
                    100_000_000_000,
                    json!({}),
                ),
                m1_out,
                m1_in,
                m2_out,
                m2_in,
                row(
                    "lbuy",
                    "L",
                    "inbound",
                    "2024-01-01",
                    10_000_000_000,
                    json!({"asset": "LBTC"}),
                ),
                lm_out,
                lm_in,
            ],
            vec![pair("m1"), pair("m2"), pair("lm")],
        ))
    }

    /// A moving-average book: two purchases in two wallets, then sales that
    /// empty the first lot and one wallet.
    fn moving_average_book() -> Value {
        run(&request(
            json!("moving_average"),
            "MOVING_AVERAGE",
            vec![
                row(
                    "b1",
                    "A",
                    "inbound",
                    "2024-01-01",
                    100_000_000_000,
                    json!({"rate": "30000E0"}),
                ),
                row(
                    "b2",
                    "B",
                    "inbound",
                    "2024-01-02",
                    200_000_000_000,
                    json!({"rate": "45000E0"}),
                ),
                row(
                    "s1",
                    "A",
                    "outbound",
                    "2024-02-01",
                    60_000_000_000,
                    json!({"rate": "50000E0"}),
                ),
                row(
                    "s2",
                    "B",
                    "outbound",
                    "2024-03-01",
                    200_000_000_000,
                    json!({"rate": "55000E0"}),
                ),
            ],
            vec![],
        ))
    }

    /// `(type, row, quantity, proceeds, cost basis, gain, holding period)`
    /// of each realized entry; `(type, row, quantity, value, unit cost)` of
    /// each acquisition; `(type, audit row)` of each transfer leg.
    fn summary(response: &Value) -> Vec<Value> {
        response["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| {
                let row = entry["row"]
                    .as_str()
                    .map(|id| id.trim_start_matches("custody-tax:"));
                match entry["entry_type"].as_str().expect("type") {
                    "acquisition" => json!([
                        "acquisition",
                        row,
                        entry["quantity"],
                        entry["fiat_value"],
                        entry["unit_cost"]
                    ]),
                    "transfer_out" | "transfer_in" => json!([entry["entry_type"], entry["audit"]]),
                    other => json!([
                        other,
                        row,
                        entry["quantity"],
                        entry["proceeds"],
                        entry["cost_basis"],
                        entry["gain_loss"],
                        entry["capital_gains_type"]
                    ]),
                }
            })
            .collect()
    }

    #[test]
    fn acquisitions_come_in_lot_order_at_their_basis_with_their_unit_cost() {
        let response = fifo_book();
        let entries = summary(&response);
        assert_eq!(
            entries[..3],
            [
                json!(["acquisition", "old", "1E0", "30000E0", "30000E0"]),
                // 100000 / 3, one 32-digit division.
                json!([
                    "acquisition",
                    "new",
                    "3E0",
                    "100000E0",
                    "33333333333333333333333333333333E-27"
                ]),
                json!(["acquisition", "earn", "1E-1", "40000E-1", "40000E0"]),
            ]
        );
        let first = &response["entries"][0];
        assert_eq!(first["wallet"], "Cold");
        assert_eq!(first["asset"], "BTC");
        assert_eq!(first["description"], "old");
    }

    #[test]
    fn realized_entries_sum_their_fragments_by_event_and_holding_period() {
        let response = fifo_book();
        assert_eq!(
            summary(&response)[3..],
            [
                json!(["disposal", "sale23", "-2E-1", "70000E-1", "60000E-1", "10000E-1", "long"]),
                // Income keeps its sign.
                json!(["income", "earn", "1E-1", "40000E-1", "0E0", "40000E-1", "short"]),
                // One sale across a long-held and a recent lot: two entries,
                // in the order the lot engine matched them.
                json!([
                    "disposal",
                    "sell",
                    "-8E-1",
                    "39973351099267155229846768820786E-27",
                    "240000E-1",
                    "15973351099267155229846768820786E-27",
                    "long"
                ]),
                json!([
                    "disposal",
                    "sell",
                    "-701E-3",
                    "35026648900732844770153231179214E-27",
                    "23366666666666666666666666666667E-27",
                    "11659982234066178103486564512547E-27",
                    "short"
                ]),
                // A fee-only spend realizes nothing: its proceeds are its
                // basis, and its gain is zero.
                json!([
                    "fee",
                    "feeonly",
                    "-1E-4",
                    "33333333333333333333333333333333E-31",
                    "33333333333333333333333333333333E-31",
                    "0E0",
                    "short"
                ]),
            ]
        );
        // The moving average's two fragments of one sale sum from zero.
        let average = moving_average_book();
        assert_eq!(
            summary(&average)[3],
            json!([
                "disposal",
                "s2",
                "-20E-1",
                "1100000E-1",
                "800000E-1",
                "300000E-1",
                "short"
            ])
        );
        assert_eq!(
            average["entries"][3]["fragments"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn fragments_carry_their_values_and_their_event_and_lot() {
        let response = fifo_book();
        let sale = &response["entries"][6];
        assert_eq!(sale["at_category"], Value::Null);
        assert_eq!(
            sale["fragments"],
            json!([{
                "crypto_amount": "701E-3",
                "cost_basis": "23366666666666666666666666666667E-27",
                "proceeds": "35026648900732844770153231179214E-27",
                "gain_loss": "11659982234066178103486564512547E-27",
                "unit_basis_override": null,
                "event": {
                    "row": "custody-tax:sell",
                    "spot_price": "50000E0",
                    "fiat_fee": "50000E-3",
                    "crypto_fee": "1E-3",
                },
                "lot": {
                    "row": "custody-tax:new",
                    "spot_price": "33333333333333333333333333333333E-27",
                    "fiat_fee": "0E0",
                    "crypto_fee": "0E0",
                    "basis": "100000E0",
                    "crypto_in": "3E0",
                },
            }])
        );
        // Income has no lot; a moving average names its unit basis.
        assert_eq!(response["entries"][4]["fragments"][0]["lot"], Value::Null);
        let average = moving_average_book();
        assert_eq!(
            average["entries"][2]["fragments"][0]["unit_basis_override"],
            "40000E0"
        );
    }

    #[test]
    fn a_taxable_move_fee_is_a_transfer_fee_and_each_move_books_two_legs() {
        let response = moves_book();
        assert_eq!(
            summary(&response),
            [
                json!(["acquisition", "buy", "1E0", "40000E0", "40000E0"]),
                // Only the move with a fee has a taxable event.
                json!([
                    "transfer_fee",
                    "m1-out",
                    "-1E-4",
                    "40000E-4",
                    "40000E-4",
                    "0E0",
                    "short"
                ]),
                json!(["transfer_out", 0]),
                json!(["transfer_in", 0]),
                json!(["transfer_out", 1]),
                json!(["transfer_in", 1]),
                json!(["acquisition", "lbuy", "1E-1", "40000E-1", "40000E0"]),
                json!([
                    "transfer_fee",
                    "lm-out",
                    "-1E-5",
                    "40000E-5",
                    "40000E-5",
                    "0E0",
                    "short"
                ]),
                // The second asset's legs name its own audit row.
                json!(["transfer_out", 2]),
                json!(["transfer_in", 2]),
            ]
        );
        // A move's fee leaves the wallet that sent it.
        let fee_wallets: Vec<&Value> = response["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["entry_type"] == "transfer_fee")
            .map(|entry| &entry["wallet"])
            .collect();
        assert_eq!(fee_wallets, [&json!("Cold"), &json!("Liquid")]);
        assert_eq!(response["intra_audit"].as_array().unwrap().len(), 3);
        assert_eq!(response["tax_summary"], json!([]));
    }

    #[test]
    fn holdings_value_positive_balances_at_the_open_lots_average_basis() {
        let holdings = |response: &Value| -> Vec<Value> {
            response["holdings"]
                .as_array()
                .expect("holdings")
                .iter()
                .map(|holding| {
                    json!([
                        holding["asset"],
                        holding["wallet"],
                        holding["quantity"],
                        holding["cost_basis"]
                    ])
                })
                .collect()
        };
        // In balance order, per asset.
        assert_eq!(
            holdings(&moves_book()),
            [
                json!(["BTC", "Cold", "4999E-4", "199960000E-4"]),
                json!(["BTC", "Exchange", "2E-1", "80000E-1"]),
                json!(["BTC", "Hot", "3E-1", "120000E-1"]),
                json!(["LBTC", "Exchange", "5E-2", "200000E-2"]),
                json!(["LBTC", "Liquid", "4999E-5", "199960000E-5"]),
            ]
        );
        // The sold-out lot adds nothing, the pool's open basis values the
        // rest, and the emptied wallet holds nothing.
        assert_eq!(
            holdings(&moving_average_book()),
            [json!(["BTC", "Cold", "4E-1", "160000E-1"])]
        );
        assert_eq!(
            holdings(&fifo_book()),
            [json!([
                "BTC",
                "Cold",
                "23989E-4",
                "80630000000000000000000000000000E-27"
            ])]
        );
    }

    #[test]
    fn the_tax_summary_drops_fees_and_moves_and_sorts_by_year_type_and_holding_period() {
        let rows: Vec<Value> = fifo_book()["tax_summary"]
            .as_array()
            .expect("summary")
            .iter()
            .map(|row| {
                json!([
                    row["year"],
                    row["asset"],
                    row["transaction_type"],
                    row["long_term"],
                    row["quantity"],
                    row["proceeds"],
                    row["cost_basis"],
                    row["gain_loss"]
                ])
            })
            .collect();
        assert_eq!(
            rows,
            [
                json!([2023, "BTC", "sell", true, "2E-1", "70000E-1", "60000E-1", "10000E-1"]),
                json!([
                    2024,
                    "BTC",
                    "sell",
                    false,
                    "701E-3",
                    "35026648900732844770153231179214E-27",
                    "23366666666666666666666666666667E-27",
                    "11659982234066178103486564512547E-27"
                ]),
                json!([
                    2024,
                    "BTC",
                    "sell",
                    true,
                    "8E-1",
                    "39973351099267155229846768820786E-27",
                    "240000E-1",
                    "15973351099267155229846768820786E-27"
                ]),
                json!([2024, "BTC", "staking", false, "1E-1", "40000E-1", "0E0", "40000E-1"]),
            ]
        );
    }

    #[test]
    fn the_method_gate_fails_the_multi_asset_stage_once_an_asset_is_computed() {
        let buy = row(
            "buy",
            "A",
            "inbound",
            "2024-01-01",
            100_000_000_000,
            json!({}),
        );
        let failure = run(&request(Value::Null, "bogus", vec![buy], vec![]));
        assert_eq!(
            failure["error"],
            json!({
                "stage": "compute_multi",
                "inner": {"class": "AppError", "message": "Unsupported RP2 accounting method 'bogus'"},
            })
        );
        // Nothing to compute: no gate.
        let sell = row(
            "sell",
            "A",
            "outbound",
            "2024-01-01",
            100_000_000_000,
            json!({}),
        );
        let response = run(&request(Value::Null, "bogus", vec![sell], vec![]));
        assert_eq!(response["ok"], true);
        assert_eq!(response["entries"], json!([]));
        assert_eq!(response["quarantines"][0]["reason"], "insufficient_lots");
    }

    #[test]
    fn a_failing_computation_names_its_asset() {
        // An LBTC lot before 1970 is invisible to the lot engine, which then
        // has no lot to start from.
        let response = run(&request(
            json!("fifo"),
            "FIFO",
            vec![
                row(
                    "buy",
                    "A",
                    "inbound",
                    "2024-01-01",
                    100_000_000_000,
                    json!({}),
                ),
                row(
                    "lbuy",
                    "L",
                    "inbound",
                    "1969-12-31",
                    100_000_000_000,
                    json!({"asset": "LBTC"}),
                ),
            ],
            vec![],
        ));
        assert_eq!(
            response["error"],
            json!({
                "stage": "compute",
                "asset": "LBTC",
                "inner": {"class": "RuntimeError", "message": "Internal error: AVL tree has no root node"},
            })
        );
    }

    #[test]
    fn each_asset_lists_its_normalize_quarantines_then_its_gates() {
        let response = run(&request(
            json!("fifo"),
            "FIFO",
            vec![
                row(
                    "buy",
                    "A",
                    "inbound",
                    "2024-01-01",
                    100_000_000_000,
                    json!({}),
                ),
                row(
                    "unpriced",
                    "A",
                    "inbound",
                    "2024-02-01",
                    10_000_000_000,
                    json!({"rate": null}),
                ),
                row(
                    "lsell",
                    "L",
                    "outbound",
                    "2024-02-15",
                    10_000_000_000,
                    json!({"asset": "LBTC"}),
                ),
                row(
                    "sell",
                    "A",
                    "outbound",
                    "2024-03-01",
                    10_000_000_000,
                    json!({"rate": "50000E0"}),
                ),
            ],
            vec![],
        ));
        let reasons: Vec<Value> = response["quarantines"]
            .as_array()
            .expect("quarantines")
            .iter()
            .map(|item| json!([item["row"], item["reason"]]))
            .collect();
        assert_eq!(
            reasons,
            [
                json!(["custody-tax:unpriced", "missing_spot_price"]),
                json!(["custody-tax:sell", "basis_provenance_incomplete"]),
                json!(["custody-tax:lsell", "insufficient_lots"]),
            ]
        );
        // LBTC has no lot, so it is not computed and books nothing.
        assert_eq!(
            summary(&response),
            [json!(["acquisition", "buy", "1E0", "40000E0", "40000E0"])]
        );
        assert_eq!(
            response["holdings"],
            json!([{"asset": "BTC", "wallet": "Cold", "quantity": "1E0", "cost_basis": "40000E0"}])
        );
    }

    /// `(type, row, category)` of each realized entry.
    fn categories(response: &Value) -> Vec<Value> {
        response["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .filter(|entry| entry.get("at_category").is_some())
            .map(|entry| {
                let row = entry["row"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("custody-tax:");
                json!([entry["entry_type"], row, entry["at_category"]])
            })
            .collect()
    }

    /// An Austrian book: Alt and Neu lots, staking, a sale, a fee-only spend
    /// across the last Alt and the first Neu, and sales whose rows carry
    /// reviewed category and taxability overrides.
    fn austrian_book(method: &str) -> Value {
        let sale = |id: &str, date: &str, amount: u64, fields: Value| {
            let mut sale = row(
                id,
                "A",
                "outbound",
                date,
                amount,
                json!({"rate": "20000E0"}),
            );
            for (key, value) in fields.as_object().expect("fields") {
                sale[key] = value.clone();
            }
            sale
        };
        run(&austrian(
            method,
            vec![
                row(
                    "alt",
                    "A",
                    "inbound",
                    "2020-06-01",
                    100_000_000_000,
                    json!({"rate": "8000E0"}),
                ),
                row(
                    "neu",
                    "A",
                    "inbound",
                    "2022-01-01",
                    100_000_000_000,
                    json!({}),
                ),
                row(
                    "earn",
                    "A",
                    "inbound",
                    "2022-02-01",
                    10_000_000_000,
                    json!({"kind": "staking"}),
                ),
                sale("sale1", "2023-01-01", 90_000_000_000, json!({})),
                sale(
                    "feeonly",
                    "2023-02-01",
                    0,
                    json!({"fee_msat": 20_000_000_000_u64, "kind": "withdrawal"}),
                ),
                sale(
                    "swap-said",
                    "2023-03-01",
                    20_000_000_000,
                    json!({"category_override": "neu_swap", "taxability_off": true}),
                ),
                sale(
                    "cleared",
                    "2023-03-02",
                    10_000_000_000,
                    json!({"category_override": "none"}),
                ),
                sale(
                    "off",
                    "2023-03-03",
                    10_000_000_000,
                    json!({"category_override": "neu_gain", "taxability_off": true}),
                ),
                sale(
                    "chosen",
                    "2023-03-04",
                    10_000_000_000,
                    json!({"category_override": "alt_spekulation"}),
                ),
            ],
            vec![],
        ))
    }

    #[test]
    fn austrian_categories_follow_the_overrides_and_split_entries() {
        // The oracle gives the same (scratch replay on RP2, WI-9 log).
        let fifo = austrian_book("fifo");
        assert_eq!(
            categories(&fifo),
            vec![
                json!(["income", "earn", "income_capital_yield"]),
                json!(["disposal", "sale1", "alt_taxfree"]),
                // One fee across an Alt and a Neu lot: two entries, split by
                // category, which a fee then drops.
                json!(["fee", "feeonly", null]),
                json!(["fee", "feeonly", null]),
                // Taxability off keeps a non-reportable override.
                json!(["disposal", "swap-said", "neu_swap"]),
                // `none` clears the category, and so does taxability off
                // with any other override.
                json!(["disposal", "cleared", null]),
                json!(["disposal", "off", null]),
                // Any other override replaces it.
                json!(["disposal", "chosen", "alt_spekulation"]),
            ]
        );
        let fees: Vec<&Value> = fifo["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["entry_type"] == "fee")
            .map(|entry| &entry["quantity"])
            .collect();
        assert_eq!(fees, [&json!("-1E-1"), &json!("-1E-1")]);
        // Under the moving average one Neu pool takes the fee whole.
        let average = austrian_book("moving_average_at");
        assert_eq!(
            categories(&average)[1],
            json!(["disposal", "sale1", "neu_loss"])
        );
        assert_eq!(categories(&average)[2], json!(["fee", "feeonly", null]));
        // A generic book reports no category at all.
        assert!(categories(&fifo_book())
            .iter()
            .all(|entry| entry[2].is_null()));
    }

    #[test]
    fn an_austrian_classification_failure_is_raised_unwrapped() {
        // A move's text repeating `at_swap_link=` is never read under a lot
        // method until its fee is classified (golden
        // `at-usertag-move-duplicate-swap-link-fifo`).
        let [mut out, inbound] = legs("m", "A", "B", "2024-02-01", 50_000_000_000, json!({}));
        out["fee_msat"] = json!(100_000_000);
        out["text"] = json!("at_swap_link=m1 at_swap_link=m2");
        let response = run(&austrian(
            "fifo",
            vec![
                row(
                    "buy",
                    "A",
                    "inbound",
                    "2024-01-01",
                    100_000_000_000,
                    json!({}),
                ),
                out,
                inbound,
            ],
            vec![pair("m")],
        ));
        assert_eq!(
            response["error"],
            json!({
                "stage": "classify",
                "inner": {
                    "class": "ValueError",
                    "message": "Duplicate `at_swap_link=` markers in notes; only one is allowed per transaction. Found: ['at_swap_link=m1', 'at_swap_link=m2']",
                },
            })
        );
    }
}
