//! The ledger pipeline (design §4.3): the adapter's
//! `_build_finalized_ledger_state` (`tests/tax_oracle/adapter.py:2174-2323`).
//!
//! It groups the rows by asset in first-appearance order, and the moves by
//! their out leg's asset, as the adapter does (`:2209-2235`), then runs
//! each prepare pass asset by asset. So far a pass is normalize alone
//! ([`super::normalize`]), and only the `prepare` operation runs it, for a
//! generic book without a carrying-value Bitcoin rail pair, whose one
//! prepare pass is its final one. An Austrian book, and a generic one that
//! needs the rail carry, is `Unsupported` until those parts are ported.
//!
//! The book's references must resolve: unique row and wallet ids, rows of
//! known wallets, moves between two distinct rows of one asset with equal
//! amounts and no shared leg. The finalized projection and the request
//! builder guarantee each of these, so a request that breaks one is refused
//! as malformed rather than read by guesswork.

use std::collections::{HashMap, HashSet};

use crate::error::{EngineError, EngineResult};
use crate::ENGINE_VERSION;

use super::normalize::{normalize, Item, Normalized};
use super::request::{Country, LedgerRequest, Row, Wallet, SCHEMA_VERSION};
use super::response::{
    EventOut, NormalizedOut, PrepareResponse, PreparedAssetOut, QuarantineOut, Stage, StagedError,
    TransferOut,
};

/// The cross-asset pair policy the carries apply to.
const CARRYING_VALUE: &str = "carrying-value";

/// A move between two rows of one asset.
#[derive(Clone, Debug)]
pub(crate) struct MovePair<'a> {
    /// Its index in the request's `intra_pairs`.
    pub index: usize,
    /// Its legs, as row indices.
    pub out: usize,
    pub inbound: usize,
    /// Its atomic group.
    pub group_id: Option<&'a str>,
}

/// One asset's rows and moves, in projection order.
#[derive(Clone, Debug)]
pub(crate) struct AssetRows<'a> {
    pub asset: &'a str,
    pub rows: Vec<usize>,
    pub pairs: Vec<MovePair<'a>>,
}

/// A request with its references resolved.
pub(crate) struct Book<'a> {
    pub request: &'a LedgerRequest,
    pub rows: &'a [Row],
    row_by_id: HashMap<&'a str, usize>,
    wallets: HashMap<&'a str, &'a Wallet>,
    /// In the order the assets first appear among the rows.
    pub assets: Vec<AssetRows<'a>>,
}

impl<'a> Book<'a> {
    pub fn new(request: &'a LedgerRequest) -> EngineResult<Self> {
        let mut wallets = HashMap::new();
        for wallet in &request.wallets {
            if wallets.insert(wallet.id.as_str(), wallet).is_some() {
                return Err(EngineError::request(format!(
                    "wallet {:?} is listed twice",
                    wallet.id
                )));
            }
        }
        let rows = request.rows.as_slice();
        let mut row_by_id = HashMap::new();
        let mut assets: Vec<AssetRows<'a>> = Vec::new();
        let mut asset_index: HashMap<&'a str, usize> = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            if row_by_id.insert(row.id.as_str(), index).is_some() {
                return Err(EngineError::request(format!(
                    "row {:?} is listed twice",
                    row.id
                )));
            }
            if !wallets.contains_key(row.wallet_id.as_str()) {
                return Err(EngineError::request(format!(
                    "row {:?} names the unknown wallet {:?}",
                    row.id, row.wallet_id
                )));
            }
            let position = *asset_index.entry(row.asset.as_str()).or_insert_with(|| {
                assets.push(AssetRows {
                    asset: row.asset.as_str(),
                    rows: Vec::new(),
                    pairs: Vec::new(),
                });
                assets.len() - 1
            });
            assets[position].rows.push(index);
        }
        let mut legs = HashSet::new();
        for (index, pair) in request.intra_pairs.iter().enumerate() {
            let leg = |id: &str| {
                row_by_id.get(id).copied().ok_or_else(|| {
                    EngineError::request(format!("move {index} names the unknown row {id:?}"))
                })
            };
            let (out, inbound) = (leg(&pair.out)?, leg(&pair.inbound)?);
            let (out_row, in_row) = (&rows[out], &rows[inbound]);
            if out_row.asset != in_row.asset {
                return Err(EngineError::request(format!("move {index} crosses assets")));
            }
            if out_row.amount_msat != in_row.amount_msat {
                return Err(EngineError::request(format!(
                    "move {index} has legs of unequal amounts"
                )));
            }
            // A row that is both legs of one move shares a leg too.
            if !legs.insert(out) || !legs.insert(inbound) {
                return Err(EngineError::request(format!(
                    "move {index} shares a leg with another move"
                )));
            }
            assets[asset_index[out_row.asset.as_str()]]
                .pairs
                .push(MovePair {
                    index,
                    out,
                    inbound,
                    group_id: pair.group_id.as_deref(),
                });
        }
        Ok(Book {
            request,
            rows,
            row_by_id,
            wallets,
            assets,
        })
    }

    /// A wallet's label; every row's wallet is known.
    pub fn wallet_label(&self, wallet_id: &str) -> Option<&'a str> {
        self.wallets
            .get(wallet_id)
            .and_then(|wallet| wallet.label.as_deref())
    }

    /// Whether the generic rail carry runs: a carrying-value Bitcoin rail
    /// pair with both legs among the rows (`adapter.py:2073-2080`).
    fn has_rail_carry(&self) -> bool {
        self.request.cross_asset_pairs.iter().any(|pair| {
            pair.policy.as_deref() == Some(CARRYING_VALUE)
                && pair.bitcoin_rail
                && self.row_by_id.contains_key(pair.out_id.as_str())
                && self.row_by_id.contains_key(pair.in_id.as_str())
        })
    }
}

/// One prepare pass over every asset, in order (`_prepare_assets`,
/// `adapter.py:1671-1706`); so far normalize alone, over all of each
/// asset's rows. The first failure ends the pass, as it does the adapter's.
fn prepare_assets(book: &Book) -> EngineResult<Vec<Normalized>> {
    book.assets
        .iter()
        .map(|part| normalize(book, part, &part.rows))
        .collect()
}

/// The `prepare` operation (design §3.10): the final prepare pass, per
/// asset. A failure inside the pass is the `prepare` stage's, raised bare.
pub(crate) fn prepare(request: &LedgerRequest) -> Result<PrepareResponse, StagedError> {
    let book = Book::new(request).map_err(StagedError::contract)?;
    match request.policy.country {
        Country::At {} => {
            return Err(StagedError::contract(EngineError::unsupported(
                "the prepare operation does not implement Austrian books yet",
            )))
        }
        Country::Generic { .. } if book.has_rail_carry() => {
            return Err(StagedError::contract(EngineError::unsupported(
                "the prepare operation does not implement the generic Bitcoin rail carry yet",
            )))
        }
        Country::Generic { .. } => {}
    }
    let normalized =
        prepare_assets(&book).map_err(|error| StagedError::new(Stage::Prepare, error))?;
    Ok(PrepareResponse {
        schema_version: SCHEMA_VERSION,
        ok: true,
        engine_version: ENGINE_VERSION,
        assets: book
            .assets
            .iter()
            .zip(normalized)
            .map(|(part, normalized)| PreparedAssetOut {
                asset: part.asset.to_owned(),
                normalized: normalized_out(&book, normalized),
            })
            .collect(),
    })
}

/// The normalized stage as the response names it: rows by id.
fn normalized_out(book: &Book, normalized: Normalized) -> NormalizedOut {
    let id = |index: usize| book.rows[index].id.clone();
    NormalizedOut {
        events: normalized
            .events
            .into_iter()
            .map(|event| EventOut {
                row: id(event.row),
                wallet_label: event.wallet_label,
                amount: event.amount.into(),
                fee: event.fee.into(),
                spot_price: event.spot_price,
                fiat_value: event.fiat_value,
                description: event.description,
                at_regime: event.at_regime,
                at_regime_basis: event.at_regime_basis,
                cost_basis_pool_id: event.cost_basis_pool_id,
                at_swap_link: event.at_swap_link,
            })
            .collect(),
        transfers: normalized
            .transfers
            .into_iter()
            .map(|transfer| TransferOut {
                pair: transfer.pair,
                out: id(transfer.out),
                inbound: id(transfer.inbound),
                from_wallet_label: transfer.from_wallet_label,
                to_wallet_label: transfer.to_wallet_label,
                sent: transfer.sent.into(),
                received: transfer.received.into(),
                fee: transfer.fee.into(),
                spot_price: transfer.spot_price,
                description: transfer.description,
                from_cost_basis_pool_id: transfer.from_cost_basis_pool_id,
                to_cost_basis_pool_id: transfer.to_cost_basis_pool_id,
                at_regime: transfer.at_regime,
                regime_flows: transfer.regime_flows,
                group_id: transfer.group_id,
            })
            .collect(),
        ordered_items: normalized
            .ordered_items
            .into_iter()
            .map(|item| match item {
                Item::Event(row) => ("event", id(row)),
                Item::Transfer(out) => ("transfer", id(out)),
            })
            .collect(),
        quarantines: normalized
            .quarantines
            .into_iter()
            .map(|quarantine| QuarantineOut {
                row: id(quarantine.row),
                reason: quarantine.reason,
                detail_kind: quarantine.detail_kind,
                detail: quarantine.detail,
            })
            .collect(),
        earliest_lot_contamination_at: normalized.earliest_lot_contamination_at,
    }
}
