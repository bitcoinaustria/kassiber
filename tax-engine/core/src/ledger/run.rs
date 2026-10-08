//! The ledger pipeline (design §4.3): the adapter's
//! `_build_finalized_ledger_state` (`tests/tax_oracle/adapter.py:2174-2323`).
//!
//! It groups the rows by asset in first-appearance order, and the moves by
//! their out leg's asset, as the adapter does (`:2209-2235`), then runs
//! each prepare pass asset by asset: normalize ([`super::normalize`]), then
//! the walk ([`super::walk`]), so the first failure is the adapter's (it
//! normalizes and walks one asset before the next). On an Austrian book the
//! swap-link selection ([`super::swaplinks`]) reads the first pass; when it
//! links or blocks a pair, a second pass leaves the blocked pairs' rows out
//! and gives the linked rows' events their links (`:2236-2266`), and its
//! stages replace the first pass's. The selection's quarantines come
//! before the final pass's. After the final pass the `ledger` operation
//! runs the lot engine over the assets with an inbound entry, as the
//! adapter does (`:1282-1379`, `:1709-1726`):
//!
//! 1. the country's checks of every such asset (`swaps::validate`; only
//!    Austria checks anything), failing at stage `validate`;
//! 2. when there is such an asset, the method gate: a profile method the
//!    policy does not allow fails at stage `compute_multi`, as the adapter
//!    builds its accounting method inside the multi-asset hook
//!    (`:620-636`, design §3.9);
//! 3. the multi-asset hook (`swaps::compute_multi`, stage `compute_multi`),
//!    which only an Austrian book whose entries' notes pair `at_swap_link=`
//!    markers takes (the runner's results are the assets' own, in order),
//!    and otherwise each asset's own computation (`compute_asset`, stage
//!    `compute`, naming the asset);
//!
//! and then composes each computed asset ([`super::compose`]) in order. An
//! asset without an inbound entry is not computed: only its quarantines and
//! transfer audit appear. Both operations run every Austrian book and a
//! generic book without a carrying-value Bitcoin rail pair; a generic book
//! that needs the rail carry is `Unsupported` until the carry is ported.
//! The notes carry the selected links to the lot engine, and marker tokens
//! a user typed into a row's text still pair swaps there, as they do today
//! (design §4.2).
//!
//! The book's references must resolve: unique row and wallet ids, rows of
//! known wallets, moves between two distinct rows of one asset with equal
//! amounts and no shared leg. The finalized projection and the request
//! builder guarantee each of these, so a request that breaks one is refused
//! as malformed rather than read by guesswork.

use std::collections::{HashMap, HashSet};

use crate::engine::{self, compute_asset};
use crate::error::{EngineError, EngineResult};
use crate::model::{AssetInput, AssetOutput, ConfigSpec, CountrySpec};
use crate::swaps;
use crate::ENGINE_VERSION;

use super::compose::{compose, ComputedAsset};
use super::normalize::{normalize, Item, Normalized, Quarantine};
use super::request::{Country, LedgerRequest, Policy, Row, Wallet, SCHEMA_VERSION};
use super::response::{
    EntryInputOut, EventOut, Inner, InnerClass, LedgerResponse, NormalizedOut, PrepareFailure,
    PrepareResponse, PreparedAssetOut, PreparedOut, QuarantineOut, Stage, StagedError,
    SwapLinksOut, TransferOut,
};
use super::swaplinks::{self, Selection};
use super::walk::{walk, Walked};

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

    /// The row with id `id`, if there is one.
    pub fn row_index(&self, id: &str) -> Option<usize> {
        self.row_by_id.get(id).copied()
    }

    /// Whether a carrying-value Bitcoin rail pair has both legs among the
    /// rows: the pairs the generic rail carry considers
    /// (`adapter.py:2073-2080`).
    fn has_carrying_rail_pair(&self) -> bool {
        self.request.cross_asset_pairs.iter().any(|pair| {
            pair.policy.as_deref() == Some(CARRYING_VALUE)
                && pair.bitcoin_rail
                && self.row_by_id.contains_key(pair.out_id.as_str())
                && self.row_by_id.contains_key(pair.in_id.as_str())
        })
    }

    /// Refuses a book the pipeline does not implement yet: a generic one
    /// that runs the rail carry.
    fn check_supported(&self, operation: &str) -> EngineResult<()> {
        match self.request.policy.country {
            Country::At {} => Ok(()),
            Country::Generic { .. } if self.has_carrying_rail_pair() => {
                Err(EngineError::unsupported(format!(
                    "the {operation} operation does not implement the generic Bitcoin rail carry yet"
                )))
            }
            Country::Generic { .. } => Ok(()),
        }
    }
}

/// One asset's stages after a prepare pass.
struct Stages {
    normalized: Normalized,
    /// `None` when the walk raised.
    walked: Option<Walked>,
}

/// What a prepare pass leaves out and links (`_prepare_assets`'
/// `excluded_row_ids` and `at_swap_link_by_row_id`).
#[derive(Default)]
struct Pass {
    /// Rows the pass does not visit.
    excluded: HashSet<usize>,
    /// The swap link of each linked row.
    links: HashMap<usize, String>,
}

/// One prepare pass over every asset, in order (`_prepare_assets`,
/// `adapter.py:1671-1706`): normalize, then walk, over each asset's rows
/// the pass does not leave out. The stages it reached, and the failure that
/// ended it, as the adapter's first failure ends its pass.
fn prepare_assets(
    book: &Book,
    config: &ConfigSpec,
    pass: &Pass,
) -> (Vec<Stages>, Option<EngineError>) {
    let mut stages = Vec::new();
    for part in &book.assets {
        let active: Vec<usize> = part
            .rows
            .iter()
            .copied()
            .filter(|row| !pass.excluded.contains(row))
            .collect();
        let normalized = match normalize(book, part, &active, &pass.links) {
            Ok(normalized) => normalized,
            Err(error) => return (stages, Some(error)),
        };
        match walk(book, part, &normalized, config) {
            Ok(walked) => stages.push(Stages {
                normalized,
                walked: Some(walked),
            }),
            Err(error) => {
                stages.push(Stages {
                    normalized,
                    walked: None,
                });
                return (stages, Some(error));
            }
        }
    }
    (stages, None)
}

/// The final prepare pass (`adapter.py:2236-2266`), the stages it reached,
/// and the swap-link selection when the first pass succeeded.
struct FinalPass {
    stages: Vec<Stages>,
    selection: Option<Selection>,
    error: Option<EngineError>,
}

/// The first pass, the swap-link selection over it (which selects nothing
/// on a generic book), and, when the selection links or blocks a pair, a
/// second pass whose stages replace the first's. The first failure ends
/// it, as it ends the adapter's run.
fn final_pass(book: &Book, config: &ConfigSpec) -> FinalPass {
    let (stages, error) = prepare_assets(book, config, &Pass::default());
    if error.is_some() {
        return FinalPass {
            stages,
            selection: None,
            error,
        };
    }
    let first_pass = stages.iter().filter_map(|stages| {
        stages
            .walked
            .as_ref()
            .map(|walked| (&stages.normalized, walked))
    });
    let selection = match swaplinks::select(book, first_pass) {
        Ok(selection) => selection,
        Err(error) => {
            return FinalPass {
                stages,
                selection: None,
                error: Some(error),
            }
        }
    };
    if !selection.changes_the_pass() {
        return FinalPass {
            stages,
            selection: Some(selection),
            error: None,
        };
    }
    let pass = Pass {
        // A blocked id no row has (a direct payout's `<pair>:out` without
        // that row) leaves nothing out.
        excluded: selection
            .blocked
            .iter()
            .filter_map(|id| book.row_index(id))
            .collect(),
        links: selection.links.iter().cloned().collect(),
    };
    let (stages, error) = prepare_assets(book, config, &pass);
    FinalPass {
        stages,
        selection: Some(selection),
        error,
    }
}

/// The `prepare` operation (design §3.10): the final prepare pass, per
/// asset, and the swap-link selection that decided it. A failure inside a
/// pass is the `prepare` stage's, raised bare, and lists the stages the
/// failing pass reached, and the selection when the first pass succeeded.
pub(crate) fn prepare(request: &LedgerRequest) -> Result<PrepareResponse, PrepareFailure> {
    let contract = |error| PrepareFailure::new(StagedError::contract(error), Vec::new(), None);
    let book = Book::new(request).map_err(contract)?;
    book.check_supported("prepare").map_err(contract)?;
    let FinalPass {
        stages,
        selection,
        error,
    } = final_pass(&book, &request.config.spec());
    let assets = book
        .assets
        .iter()
        .zip(stages)
        .map(|(part, stages)| PreparedAssetOut {
            asset: part.asset.to_owned(),
            normalized: normalized_out(&book, stages.normalized),
            prepared: stages.walked.map(|walked| prepared_out(&book, walked)),
        })
        .collect();
    let swap_links = selection.map(|selection| swap_links_out(&book, selection));
    match error {
        // A final pass that did not fail always has its selection.
        None => Ok(PrepareResponse {
            schema_version: SCHEMA_VERSION,
            ok: true,
            engine_version: ENGINE_VERSION,
            assets,
            swap_links: swap_links.unwrap_or_default(),
        }),
        Some(error) => Err(PrepareFailure::new(
            StagedError::new(Stage::Prepare, error),
            assets,
            swap_links,
        )),
    }
}

/// The `ledger` operation (design §4.3): the final prepare pass, the lot
/// engine over every asset with an inbound entry, and the composed ledger.
/// A failure names the stage it happened at (design §3.9).
pub(crate) fn ledger(request: &LedgerRequest) -> Result<LedgerResponse, StagedError> {
    let book = Book::new(request).map_err(StagedError::contract)?;
    book.check_supported("ledger")
        .map_err(StagedError::contract)?;
    let FinalPass {
        stages,
        selection,
        error,
    } = final_pass(&book, &request.config.spec());
    if let Some(error) = error {
        return Err(StagedError::new(Stage::Prepare, error));
    }
    // A final pass that did not fail always has its selection.
    let selection = selection.unwrap_or_default();
    // The assets the lot engine computes, in order.
    let inputs: Vec<AssetInput> = book
        .assets
        .iter()
        .zip(&stages)
        .filter_map(|(part, stages)| {
            let entries = stages.walked.as_ref()?.entries.as_ref()?;
            Some(AssetInput {
                asset: part.asset.to_owned(),
                entries: entries.iter().map(|built| built.input.clone()).collect(),
            })
        })
        .collect();
    let country = engine_country(&request.policy.country);
    swaps::validate(&inputs, country).map_err(|error| StagedError::new(Stage::Validate, error))?;
    let mut outputs = compute(&request.policy, &inputs, country)?.into_iter();
    let mut response = LedgerResponse {
        schema_version: SCHEMA_VERSION,
        ok: true,
        engine_version: ENGINE_VERSION,
        entries: Vec::new(),
        // The swap pairs' quarantines come before the assets'.
        quarantines: selection
            .quarantines
            .into_iter()
            .map(|quarantine| quarantine_out(&book, quarantine))
            .collect(),
        intra_audit: Vec::new(),
        tax_summary: Vec::new(),
        holdings: Vec::new(),
        carried_rows: Vec::new(),
    };
    // Composition, asset by asset. Its own invariant failures (impossible
    // after a successful prepare pass and computation) are composition
    // failures too.
    let internal = |message| StagedError::new(Stage::Classify, EngineError::runtime(message));
    for stages in stages {
        let walked = stages
            .walked
            .ok_or_else(|| internal("Internal error: a prepared asset was not walked"))?;
        if let Some(entries) = &walked.entries {
            let output = outputs.next().ok_or_else(|| {
                internal("Internal error: the lot engine computed fewer assets than it was given")
            })?;
            let asset = ComputedAsset {
                entries,
                output: &output,
                audit: &walked.audit,
                first_audit: response.intra_audit.len(),
            };
            // Composition raises what the adapter's composition raised,
            // unwrapped.
            let composed =
                compose(&book, &asset).map_err(|error| StagedError::new(Stage::Classify, error))?;
            response.entries.extend(composed.entries);
            response.holdings.extend(composed.holdings);
            response.tax_summary.extend(composed.tax_summary);
        }
        // The asset's quarantines: normalize's, then the gates'.
        response.quarantines.extend(
            stages
                .normalized
                .quarantines
                .into_iter()
                .chain(walked.quarantines)
                .map(|quarantine| quarantine_out(&book, quarantine)),
        );
        response.intra_audit.extend(walked.audit);
    }
    Ok(response)
}

/// The lot engine's country.
fn engine_country(country: &Country) -> engine::Country {
    engine::Country::from_spec(&match country {
        Country::Generic { long_term_days } => CountrySpec::Generic {
            long_term_days: *long_term_days,
        },
        Country::At {} => CountrySpec::At {},
    })
}

/// The method gate (`_build_rp2_accounting_engine`, `adapter.py:620-636`):
/// the adapter's own `AppError`, raised inside the multi-asset hook.
fn method_gate(policy: &Policy) -> StagedError {
    StagedError {
        stage: Stage::ComputeMulti,
        asset: None,
        inner: Inner {
            class: InnerClass::AppError,
            message: format!(
                "Unsupported RP2 accounting method '{}'",
                policy.method_label
            ),
        },
    }
}

/// The lot engine over `inputs`, the assets with an inbound entry, in
/// order (`_rp2_asset_states_from_prepared`, `adapter.py:1321-1379`): the
/// method gate and the multi-asset hook (Austria's swap runner, when the
/// entries' notes pair swap markers), else each asset on its own. One
/// result per input, in order.
fn compute(
    policy: &Policy,
    inputs: &[AssetInput],
    country: engine::Country,
) -> Result<Vec<AssetOutput>, StagedError> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let method = policy
        .method
        .ok_or_else(|| method_gate(policy))?
        .method_name();
    let multi = swaps::compute_multi(inputs, country, method)
        .map_err(|error| StagedError::new(Stage::ComputeMulti, error))?;
    if let Some(outputs) = multi {
        // The adapter takes each asset's result from the hook's mapping
        // (`:1355-1371`). The runner returns one per input, in order, so no
        // asset can be missing from it.
        let complete = outputs.len() == inputs.len()
            && outputs
                .iter()
                .zip(inputs)
                .all(|(output, input)| output.asset == input.asset);
        if !complete {
            return Err(StagedError::new(
                Stage::Classify,
                EngineError::runtime(
                    "Internal error: the multi-asset runner did not compute every asset",
                ),
            ));
        }
        return Ok(outputs);
    }
    inputs
        .iter()
        .map(|input| {
            compute_asset(input, country, method).map_err(|error| StagedError {
                asset: Some(input.asset.clone()),
                ..StagedError::new(Stage::Compute, error)
            })
        })
        .collect()
}

/// The swap-link selection as the response names it: rows by id, the
/// blocked ids sorted.
fn swap_links_out(book: &Book, selection: Selection) -> SwapLinksOut {
    let id = |index: usize| book.rows[index].id.clone();
    SwapLinksOut {
        links: selection
            .links
            .into_iter()
            .map(|(row, link)| (id(row), link))
            .collect(),
        blocked: selection.blocked.into_iter().collect(),
        quarantines: selection
            .quarantines
            .into_iter()
            .map(|quarantine| quarantine_out(book, quarantine))
            .collect(),
    }
}

/// A quarantine as the response names it: its row by id.
fn quarantine_out(book: &Book, quarantine: Quarantine) -> QuarantineOut {
    QuarantineOut {
        row: book.rows[quarantine.row].id.clone(),
        reason: quarantine.reason,
        detail_kind: quarantine.detail_kind,
        detail: quarantine.detail,
    }
}

/// The prepared stage as the response names it: each entry with its row's
/// timestamp, rows by id.
fn prepared_out(book: &Book, walked: Walked) -> PreparedOut {
    PreparedOut {
        entries: walked.entries.map(|entries| {
            entries
                .into_iter()
                .map(|entry| EntryInputOut {
                    ts: book.rows[entry.row].ts.clone(),
                    input: entry.input,
                })
                .collect()
        }),
        quarantines: walked
            .quarantines
            .into_iter()
            .map(|quarantine| quarantine_out(book, quarantine))
            .collect(),
        intra_audit: walked.audit,
    }
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
            .map(|quarantine| quarantine_out(book, quarantine))
            .collect(),
        earliest_lot_contamination_at: normalized.earliest_lot_contamination_at,
    }
}
