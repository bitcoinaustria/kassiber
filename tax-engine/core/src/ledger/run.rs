//! The ledger pipeline (design §4.3): the adapter's
//! `_build_finalized_ledger_state` (`tests/tax_oracle/adapter.py:2174-2323`).
//!
//! It groups the rows by asset in first-appearance order, and the moves by
//! their out leg's asset, as the adapter does (`:2209-2235`), then runs
//! each prepare pass asset by asset: normalize ([`super::normalize`]), then
//! the walk ([`super::walk`]), so the first failure is the adapter's (it
//! normalizes and walks one asset before the next). On a generic book the
//! Bitcoin rail carry ([`super::carry`]) runs first, its own passes with the
//! lot engine included; the passes after it read the rows it priced and
//! leave out the rows its blocked pairs withhold (`:2217-2244`). On an
//! Austrian book the swap-link selection ([`super::swaplinks`]) reads the
//! first pass; when it links or blocks a pair, a second pass also leaves
//! the blocked pairs' rows out and gives the linked rows' events their
//! links (`:2245-2266`), and its stages replace the first pass's. The
//! carry's quarantines come first, then the selection's, then the final
//! pass's. After the final pass the `ledger` operation runs the lot engine
//! over the assets with an inbound entry, as the adapter does
//! (`:1282-1379`, `:1709-1726`):
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
//! transfer audit appear. Both operations run every book. The notes carry
//! the selected links to the lot engine, and marker tokens a user typed
//! into a row's text still pair swaps there, as they do today (design
//! §4.2).
//!
//! The book's references must resolve: unique row and wallet ids, rows of
//! known wallets, moves between two distinct rows of one asset with equal
//! amounts and no shared leg. The finalized projection and the request
//! builder guarantee each of these, so a request that breaks one is refused
//! as malformed rather than read by guesswork.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::engine::{self, compute_parsed, ParsedAsset};
use crate::error::{EngineError, EngineResult};
use crate::manifest::NON_SALE_DISPOSAL_KIND_TOKENS;
use crate::model::{AssetOutput, BelowMarket, ConfigSpec, CountrySpec};
use crate::par;
use crate::swaps;
use crate::ENGINE_VERSION;

use super::carry::{carry, Carried, Carry, CarryFailure, Settled};
use super::compose::{compose, ComputedAsset};
use super::normalize::{normalize, Item, Normalized, Quarantine};
use super::request::{Country, Direction, LedgerRequest, Policy, Row, Wallet, SCHEMA_VERSION};
use super::response::{
    CarriedRow, EntryInputOut, EventOut, Inner, InnerClass, LedgerResponse, NormalizedOut,
    PrepareFailure, PrepareResponse, PreparedAssetOut, PreparedOut, QuarantineOut, RailCarryOut,
    Stage, StagedError, SwapLinksOut, TransferOut,
};
use super::swaplinks::{self, Selection};
use super::walk::{walk, Walked};

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
    /// The lot engine's country.
    pub country: engine::Country,
    /// The legs of each taxable cross-asset exchange, by row: the partner
    /// leg, and whether the row is the given (outbound) leg. Only a country
    /// whose swap values are not each leg's own reads them.
    pub swap_partners: HashMap<usize, (usize, bool)>,
    /// A rules country's kind rules and below-market valuation; the
    /// generic rules for every other country.
    pub kinds: KindRules,
}

/// What a rules country decides by a row's kind, and how it values a sale
/// below market value (`docs/reference/tax-engine.md#rules-countries`).
#[derive(Clone, Debug, Default)]
pub(crate) struct KindRules {
    /// Inbound kinds booked at zero cost, with no income.
    pub zero_cost: Vec<String>,
    pub below_market: BelowMarket,
}

impl KindRules {
    fn from_country(country: &Country) -> EngineResult<Self> {
        let Country::Rules(spec) = country else {
            return Ok(KindRules::default());
        };
        // An outbound refusal is held as a non-sale disposal, so it may name
        // only such kinds.
        for refused in &spec.refused_receipts {
            if refused.direction == Direction::Outbound {
                if let Some(kind) = refused
                    .kinds
                    .iter()
                    .find(|kind| !NON_SALE_DISPOSAL_KIND_TOKENS.contains(&kind.as_str()))
                {
                    return Err(EngineError::request(format!(
                        "the country rules refuse the outbound kind {kind:?}, which is not a non-sale disposal"
                    )));
                }
            }
        }
        Ok(KindRules {
            zero_cost: spec.zero_cost_kinds.clone(),
            below_market: spec.below_market,
        })
    }

    /// Whether an inbound row of `kind` is a zero-cost acquisition.
    pub fn zero_cost(&self, direction: Direction, kind: &str) -> bool {
        direction == Direction::Inbound && self.zero_cost.iter().any(|zero| zero == kind)
    }
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
        let country = engine_country(&request.policy.country)?;
        let kinds = KindRules::from_country(&request.policy.country)?;
        let mut swap_partners = HashMap::new();
        if !country.swap_values().is_own() {
            // A pair that is not a carrying-value one books its legs as a
            // disposal and an acquisition. A leg of two pairs has no single
            // partner and keeps its own value.
            let mut seen = HashSet::new();
            for pair in &request.cross_asset_pairs {
                if pair.policy.as_deref() == Some(super::swaplinks::CARRYING_VALUE) {
                    continue;
                }
                let (Some(&out), Some(&inbound)) = (
                    row_by_id.get(pair.out_id.as_str()),
                    row_by_id.get(pair.in_id.as_str()),
                ) else {
                    continue;
                };
                if rows[out].direction != Direction::Outbound
                    || rows[inbound].direction != Direction::Inbound
                {
                    continue;
                }
                for (row, partner, given) in [(out, inbound, true), (inbound, out, false)] {
                    if !seen.insert(row) {
                        swap_partners.remove(&row);
                    } else {
                        swap_partners.insert(row, (partner, given));
                    }
                }
            }
        }
        Ok(Book {
            request,
            rows,
            row_by_id,
            wallets,
            assets,
            country,
            swap_partners,
            kinds,
        })
    }

    /// A wallet's label; every row's wallet is known.
    /// The refusal of a row of `kind` in `direction`, when the request's
    /// rules country refuses that kind.
    pub fn refused_receipt(
        &self,
        direction: Direction,
        kind: &str,
    ) -> Option<&'a crate::model::RefusedReceipt> {
        match &self.request.policy.country {
            Country::Rules(rules) => rules.refused_receipt(direction, kind),
            _ => None,
        }
    }

    pub fn wallet_label(&self, wallet_id: &str) -> Option<&'a str> {
        self.wallets
            .get(wallet_id)
            .and_then(|wallet| wallet.label.as_deref())
    }

    /// The row with id `id`, if there is one.
    pub fn row_index(&self, id: &str) -> Option<usize> {
        self.row_by_id.get(id).copied()
    }

    /// The rows with the given ids; an id no row has leaves nothing out.
    fn rows_of<'i>(&self, ids: impl IntoIterator<Item = &'i String>) -> HashSet<usize> {
        ids.into_iter()
            .filter_map(|id| self.row_index(id))
            .collect()
    }
}

/// One asset's stages after a prepare pass.
pub(crate) struct Stages {
    pub normalized: Normalized,
    /// `None` when the walk raised.
    pub walked: Option<Walked>,
}

/// What a prepare pass leaves out and links (`_prepare_assets`'
/// `excluded_row_ids` and `at_swap_link_by_row_id`).
#[derive(Default)]
pub(crate) struct Pass {
    /// Rows the pass does not visit.
    pub excluded: HashSet<usize>,
    /// The swap link of each linked row.
    pub links: HashMap<usize, String>,
}

/// One prepare pass over every asset, in order (`_prepare_assets`,
/// `adapter.py:1671-1706`): normalize, then walk, over each asset's rows
/// the pass does not leave out. The stages it reached, and the failure that
/// ended it, as the adapter's first failure ends its pass. Each asset's
/// stages depend only on the book and the pass, so the assets are prepared
/// concurrently and taken in order, up to the first failure.
pub(crate) fn prepare_assets(
    book: &Book,
    config: &ConfigSpec,
    pass: &Pass,
) -> (Vec<Stages>, Option<EngineError>) {
    let _span = crate::timing::span("prepare");
    let prepared = par::map(&book.assets, 1, |part| {
        let active: Vec<usize> = part
            .rows
            .iter()
            .copied()
            .filter(|row| !pass.excluded.contains(row))
            .collect();
        let normalized = normalize(book, part, &active, &pass.links)?;
        let walked = walk(book, part, &normalized, config);
        Ok((normalized, walked))
    });
    let mut stages = Vec::new();
    for outcome in prepared {
        let (normalized, walked) = match par::unwrap(outcome) {
            Ok(prepared) => prepared,
            Err(error) => return (stages, Some(error)),
        };
        match walked {
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
    /// The lot engine's results on `stages`, when the carry's settled pass
    /// computed them.
    outputs: Option<Vec<AssetOutput>>,
}

/// The first pass, without the rows the rail carry withholds
/// (`carry_blocked`), the swap-link selection over it (which selects
/// nothing on a generic book), and, when the selection links or blocks a
/// pair, a second pass, also without the blocked pairs' rows, whose stages
/// replace the first's. The first failure ends it, as it ends the adapter's
/// run. The carry's settled pass, which read the same carried request, is
/// the first pass when it left out the same rows (a pass has no links
/// before the selection).
fn final_pass(
    book: &Book,
    config: &ConfigSpec,
    carry_blocked: &HashSet<usize>,
    settled: Option<Settled>,
) -> FinalPass {
    let first = Pass {
        excluded: carry_blocked.clone(),
        links: HashMap::new(),
    };
    let (stages, mut outputs) = match settled.filter(|settled| settled.excluded == first.excluded) {
        Some(settled) => (settled.stages, Some(settled.outputs)),
        None => {
            let (stages, error) = prepare_assets(book, config, &first);
            if error.is_some() {
                return FinalPass {
                    stages,
                    selection: None,
                    error,
                    outputs: None,
                };
            }
            (stages, None)
        }
    };
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
                outputs: None,
            }
        }
    };
    if !selection.changes_the_pass() {
        return FinalPass {
            stages,
            selection: Some(selection),
            error: None,
            outputs,
        };
    }
    // The second pass's stages replace the first's.
    outputs = None;
    let mut excluded = carry_blocked.clone();
    // A blocked id no row has (a direct payout's `<pair>:out` without that
    // row) leaves nothing out.
    excluded.extend(book.rows_of(&selection.blocked));
    let pass = Pass {
        excluded,
        links: selection.links.iter().cloned().collect(),
    };
    let (stages, error) = prepare_assets(book, config, &pass);
    FinalPass {
        stages,
        selection: Some(selection),
        error,
        outputs,
    }
}

/// Each asset's stages as the response names them.
fn assets_out(book: &Book, stages: Vec<Stages>) -> Vec<PreparedAssetOut> {
    book.assets
        .iter()
        .zip(stages)
        .map(|(part, stages)| PreparedAssetOut {
            asset: part.asset.to_owned(),
            normalized: normalized_out(book, stages.normalized),
            prepared: stages.walked.map(|walked| prepared_out(book, walked)),
        })
        .collect()
}

/// The `prepare` operation (design §3.10): the rail carry, the final
/// prepare pass, per asset, and the swap-link selection that decided it.
/// A failure inside a pass is the `prepare` stage's, raised bare, and
/// lists the stages the failing pass reached, the carry when it ended, and
/// the selection when the first pass succeeded. A carry pass that fails
/// fails at its own stage (its lot engine runs, with the method gate) and
/// lists that pass's stages. The failure is boxed: it carries every
/// document a success does.
pub(crate) fn prepare(request: &LedgerRequest) -> Result<PrepareResponse, Box<PrepareFailure>> {
    let contract = |error| {
        Box::new(PrepareFailure::new(
            StagedError::contract(error),
            Vec::new(),
            None,
            None,
        ))
    };
    let config = request.config.spec();
    let book = Book::new(request).map_err(contract)?;
    let mut carry = match carry(&book, &config) {
        Ok(carry) => carry,
        Err(CarryFailure { error, stages }) => {
            return Err(Box::new(PrepareFailure::new(
                error,
                assets_out(&book, stages),
                None,
                None,
            )))
        }
    };
    let carried = match carry.request(request) {
        Ok(carried) => carried,
        // Before the final pass: no stages, the carry as it ended.
        Err(error) => {
            return Err(Box::new(PrepareFailure::new(
                error,
                Vec::new(),
                None,
                Some(rail_carry_out(&book, carry)),
            )))
        }
    };
    let book = Book::new(&carried).map_err(contract)?;
    let settled = carry.take_settled();
    let FinalPass {
        stages,
        selection,
        error,
        ..
    } = final_pass(&book, &config, &book.rows_of(&carry.blocked), settled);
    let assets = assets_out(&book, stages);
    let swap_links = selection.map(|selection| swap_links_out(&book, selection));
    let rail_carry = rail_carry_out(&book, carry);
    match error {
        // A final pass that did not fail always has its selection.
        None => Ok(PrepareResponse {
            schema_version: SCHEMA_VERSION,
            ok: true,
            engine_version: ENGINE_VERSION,
            assets,
            swap_links: swap_links.unwrap_or_default(),
            rail_carry,
        }),
        Some(error) => Err(Box::new(PrepareFailure::new(
            StagedError::new(Stage::Prepare, error),
            assets,
            swap_links,
            Some(rail_carry),
        ))),
    }
}

/// The `ledger` operation (design §4.3): the rail carry, the final prepare
/// pass, the lot engine over every asset with an inbound entry, and the
/// composed ledger. A failure names the stage it happened at (design
/// §3.9).
pub(crate) fn ledger(request: &LedgerRequest) -> Result<LedgerResponse, StagedError> {
    let span = crate::timing::span("book");
    let config = request.config.spec();
    let book = Book::new(request).map_err(StagedError::contract)?;
    drop(span);
    let span = crate::timing::span("carry");
    let mut carry = carry(&book, &config).map_err(|failure| failure.error)?;
    drop(span);
    let span = crate::timing::span("carried_request");
    let carried = carry.request(request)?;
    // When the carry priced nothing, the final pass reads the request's own
    // book.
    let carried_book;
    let book = match &carried {
        Cow::Borrowed(_) => &book,
        Cow::Owned(carried) => {
            carried_book = Book::new(carried).map_err(StagedError::contract)?;
            &carried_book
        }
    };
    let settled = carry.take_settled();
    drop(span);
    let span = crate::timing::span("final_pass");
    let FinalPass {
        stages,
        selection,
        error,
        outputs,
    } = final_pass(book, &config, &book.rows_of(&carry.blocked), settled);
    if let Some(error) = error {
        return Err(StagedError::new(Stage::Prepare, error));
    }
    drop(span);
    // A final pass that did not fail always has its selection.
    let selection = selection.unwrap_or_default();
    let mut outputs = match outputs {
        // The carry's settled pass computed these stages. Its lot engine
        // ran the set checks the country's checks run, and a generic book,
        // the only one the carry passes, has no other check.
        Some(outputs) => outputs,
        None => {
            // The assets the lot engine computes, in order.
            let span = crate::timing::span("lot_inputs");
            let assets = lot_assets(book, &stages);
            drop(span);
            let span = crate::timing::span("validate");
            swaps::validate_parsed(&assets, book.country)
                .map_err(|error| StagedError::new(Stage::Validate, error))?;
            drop(span);
            let _compute = crate::timing::span("compute");
            compute(book, assets)?
        }
    }
    .into_iter();
    let _compose = crate::timing::span("compose");
    let mut response = LedgerResponse {
        schema_version: SCHEMA_VERSION,
        ok: true,
        engine_version: ENGINE_VERSION,
        entries: Vec::new(),
        // The rail carry's quarantines, then the swap pairs', come before
        // the assets'.
        quarantines: carry
            .quarantines
            .into_iter()
            .chain(selection.quarantines)
            .map(|quarantine| quarantine_out(book, quarantine))
            .collect(),
        intra_audit: Vec::new(),
        tax_summary: Vec::new(),
        holdings: Vec::new(),
        carried_rows: carried_rows_out(book, &carry.carried),
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
                compose(book, &asset).map_err(|error| StagedError::new(Stage::Classify, error))?;
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
                .map(|quarantine| quarantine_out(book, quarantine)),
        );
        response.intra_audit.extend(walked.audit);
    }
    Ok(response)
}

/// The lot engine's country; rules naming a zone without a calendar are a
/// request error.
fn engine_country(country: &Country) -> EngineResult<engine::Country> {
    engine::Country::from_spec(&match country {
        Country::Generic { long_term_days } => CountrySpec::Generic {
            long_term_days: *long_term_days,
        },
        Country::At {} => CountrySpec::At {},
        Country::Us {} => CountrySpec::Us {},
        Country::Rules(rules) => CountrySpec::Rules(rules.clone()),
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

/// The lot engine's assets: each walked asset with an inbound entry, in
/// order, with the entries the walk constructed. Their constructors ran
/// with the book's names, which only adds membership checks, so the lot
/// engine's own constructors would build the same entries; the sets'
/// checks run where the lot engine builds its sets.
pub(crate) fn lot_assets<'s>(book: &Book, stages: &'s [Stages]) -> Vec<ParsedAsset<'s>> {
    book.assets
        .iter()
        .zip(stages)
        .filter_map(|(part, stages)| {
            let entries = stages.walked.as_ref()?.entries.as_ref()?;
            Some(ParsedAsset {
                asset: part.asset.to_owned(),
                entries: Cow::Borrowed(&entries.entries),
            })
        })
        .collect()
}

/// The lot engine over `assets`, the assets with an inbound entry, in
/// order (`_rp2_asset_states_from_prepared`, `adapter.py:1321-1379`): the
/// method gate and the multi-asset hook (Austria's swap runner, when the
/// entries' notes pair swap markers), else each asset on its own. One
/// result per asset, in order.
pub(crate) fn compute(
    book: &Book,
    assets: Vec<ParsedAsset<'_>>,
) -> Result<Vec<AssetOutput>, StagedError> {
    let policy = &book.request.policy;
    let country = book.country;
    if assets.is_empty() {
        return Ok(Vec::new());
    }
    let method = policy
        .method
        .ok_or_else(|| method_gate(policy))?
        .method_name();
    let names: Vec<String> = assets.iter().map(|asset| asset.asset.clone()).collect();
    let multi = swaps::compute_multi_parsed(assets, country, method)
        .map_err(|error| StagedError::new(Stage::ComputeMulti, error))?;
    let assets = match multi {
        Ok(outputs) => {
            // The adapter takes each asset's result from the hook's mapping
            // (`:1355-1371`). The runner returns one per asset, in order, so
            // no asset can be missing from it.
            let complete = outputs.len() == names.len()
                && outputs
                    .iter()
                    .zip(&names)
                    .all(|(output, name)| output.asset == *name);
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
        Err(assets) => assets,
    };
    // Each asset's computation is its own, so they run concurrently and
    // are taken in order, up to the first failure.
    par::map(&assets, 1, |parsed| {
        let borrowed = ParsedAsset {
            asset: parsed.asset.clone(),
            entries: Cow::Borrowed(&parsed.entries),
        };
        compute_parsed(borrowed, country, method, policy.pool_scope)
    })
    .into_iter()
    .zip(&assets)
    .map(|(outcome, parsed)| {
        par::unwrap(outcome).map_err(|error| StagedError {
            asset: Some(parsed.asset.clone()),
            ..StagedError::new(Stage::Compute, error)
        })
    })
    .collect()
}

/// The carried rows as the response names them: by id, in row order.
fn carried_rows_out(book: &Book, carried: &BTreeMap<usize, Carried>) -> Vec<CarriedRow> {
    carried
        .iter()
        .map(|(&row, pricing)| CarriedRow {
            row: book.rows[row].id.clone(),
            value: pricing.value.clone(),
            rate: pricing.rate.clone(),
        })
        .collect()
}

/// The rail carry as the `prepare` response names it: the carried rows in
/// row order, the blocked ids sorted, the quarantines, and the passes.
fn rail_carry_out(book: &Book, carry: Carry) -> RailCarryOut {
    RailCarryOut {
        carried: carried_rows_out(book, &carry.carried),
        blocked: carry.blocked.into_iter().collect(),
        quarantines: carry
            .quarantines
            .into_iter()
            .map(|quarantine| quarantine_out(book, quarantine))
            .collect(),
        passes: carry.passes,
    }
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
                .built
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
