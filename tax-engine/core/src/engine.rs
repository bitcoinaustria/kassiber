//! One asset's computation (RP2's `compute_tax`).
//!
//! [`PreparedAsset`] holds the visible, ordered entry sets; [`compute_asset`]
//! runs the taxable-event pass and derives the outputs in the order RP2's
//! `ComputedData` does, so the first failure is RP2's first failure.

use std::collections::HashSet;

use crate::austria;
use crate::cursor::{Cursor, Fragment};
use crate::decimal::Decimal;
use crate::entry::Entry;
use crate::error::{EngineError, EngineResult};
use crate::methods::{lot_method, LotView};
use crate::model::{AssetInput, AssetOutput, CountrySpec, Dec, EntryKind, ErrorBody, MethodName};
use crate::num::{lt13, zero};
use crate::outputs::{
    balances, fragment_values, gain_loss_out, open_positions, sold_percentages, yearly,
};

/// The country rules the lot engine consults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Country {
    /// Kassiber's generic country.
    Generic {
        /// Whole days a lot must be held for a long-term gain.
        long_term_days: i64,
    },
    /// Austria. Holding-period and Alt/Neu semantics live in its
    /// classification hooks, not in the lot engine.
    Austria,
}

impl Country {
    /// Reads the request's country.
    pub fn from_spec(spec: &CountrySpec) -> Self {
        match spec {
            // Any period above i64::MAX days behaves identically.
            CountrySpec::Generic { long_term_days } => Country::Generic {
                long_term_days: i64::try_from(*long_term_days).unwrap_or(i64::MAX),
            },
            CountrySpec::At {} => Country::Austria,
        }
    }

    /// RP2's `get_long_term_capital_gain_period()`; Austria's is
    /// `sys.maxsize`, so it is never reached.
    pub fn long_term_days(self) -> i64 {
        match self {
            Country::Generic { long_term_days } => long_term_days,
            Country::Austria => i64::MAX,
        }
    }
}

/// One asset's constructed entries: what the adapter holds after building
/// its transaction sets, before any computation.
pub(crate) struct ParsedAsset {
    pub asset: String,
    /// Every entry, in insertion order.
    pub entries: Vec<Entry>,
}

impl ParsedAsset {
    /// Runs every constructor and the per-set duplicate-row check, failing
    /// as building RP2's transaction sets does.
    pub fn new(input: &AssetInput) -> EngineResult<Self> {
        let entries = input
            .entries
            .iter()
            .map(|entry| Entry::from_input(entry, None))
            .collect::<EngineResult<Vec<_>>>()?;
        for entry in &entries {
            if entry.asset != input.asset {
                return Err(EngineError::value(format!(
                    "Attempting to add a {} entry to a {} set",
                    entry.asset, input.asset
                )));
            }
        }
        // Each TransactionSet rejects a repeated row as it is added.
        for kind in [EntryKind::In, EntryKind::Out, EntryKind::Intra] {
            reject_duplicates(entries.iter().filter(|e| e.kind == kind))?;
        }
        Ok(ParsedAsset {
            asset: input.asset.clone(),
            entries,
        })
    }

    /// Indices of the entries of `kind` RP2's set iteration yields, in its
    /// order.
    pub fn visible(&self, kind: EntryKind) -> Vec<usize> {
        visible_sorted(&self.entries, kind)
    }
}

/// One asset's validated entries and RP2's views of them.
pub(crate) struct PreparedAsset {
    pub asset: String,
    /// Every entry, in insertion order.
    pub entries: Vec<Entry>,
    /// Visible IN, OUT, and INTRA entries, each stably sorted by instant.
    pub ins: Vec<usize>,
    pub outs: Vec<usize>,
    pub intras: Vec<usize>,
    /// The lot list (visible IN entries in order).
    pub lots: Vec<LotView>,
    /// Taxable events in processing order.
    pub events: Vec<usize>,
}

/// Indices of the visible entries of `kind`, stably sorted by instant.
fn visible_sorted(entries: &[Entry], kind: EntryKind) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].kind == kind && entries[i].ts.is_visible())
        .collect();
    indices.sort_by_key(|&i| entries[i].ts.us);
    indices
}

/// RP2's "Entry already added" check for a sequence of additions.
fn reject_duplicates<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> EngineResult<()> {
    let mut seen = HashSet::new();
    for entry in entries {
        if !seen.insert(entry.row) {
            return Err(EngineError::value(format!(
                "Entry already added: {}",
                entry.text
            )));
        }
    }
    Ok(())
}

impl PreparedAsset {
    /// Validates `input`'s entries and builds the sets RP2 iterates. Fails
    /// as RP2 does before the first taxable event: on an invalid entry, a
    /// duplicate row, or no visible lot.
    pub fn new(input: &AssetInput) -> EngineResult<Self> {
        Self::from_parsed(ParsedAsset::new(input)?)
    }

    /// Builds the views `compute_tax` iterates from constructed entries.
    pub fn from_parsed(parsed: ParsedAsset) -> EngineResult<Self> {
        let ParsedAsset { asset, entries } = parsed;
        let ins = visible_sorted(&entries, EntryKind::In);
        let outs = visible_sorted(&entries, EntryKind::Out);
        let intras = visible_sorted(&entries, EntryKind::Intra);

        // The combined set adds IN, then OUT, then INTRA entries and rejects
        // a row seen in another set; its stable sort by instant puts earn
        // receipts before disposals before move fees at one instant.
        let mut all: Vec<usize> = ins.iter().chain(&outs).chain(&intras).copied().collect();
        reject_duplicates(all.iter().map(|&i| &entries[i]))?;
        all.sort_by_key(|&i| entries[i].ts.us);
        let mut events = Vec::new();
        for index in all {
            if entries[index].is_taxable()? {
                events.push(index);
            }
        }

        let lots = ins
            .iter()
            .map(|&index| {
                let entry = &entries[index];
                let (Some(crypto_in), Some(fiat_in_with_fee)) =
                    (entry.crypto_in(), entry.fiat_in_with_fee())
                else {
                    return Err(EngineError::runtime(
                        "Internal error: lot is not an InTransaction",
                    ));
                };
                Ok(LotView {
                    entry: index,
                    row: entry.row,
                    us: entry.ts.us,
                    posix: entry.ts.posix_seconds()?,
                    crypto_in: crypto_in.clone(),
                    fiat_in_with_fee: fiat_in_with_fee.clone(),
                })
            })
            .collect::<EngineResult<Vec<_>>>()?;
        if lots.is_empty() {
            return Err(EngineError::runtime(
                "Internal error: AVL tree has no root node",
            ));
        }
        Ok(PreparedAsset {
            asset,
            entries,
            ins,
            outs,
            intras,
            lots,
            events,
        })
    }
}

/// RP2's `compute_tax` for one asset.
pub(crate) fn compute_asset(
    input: &AssetInput,
    country: Country,
    method: MethodName,
) -> EngineResult<AssetOutput> {
    let prepared = PreparedAsset::new(input)?;
    let mut cursor = Cursor::new(
        &prepared.entries,
        &prepared.lots,
        &prepared.events,
        lot_method(method, &prepared.entries, &prepared.lots),
    );
    let mut fragments = Vec::new();
    cursor.run(&mut fragments)?;
    // RP2 replays the whole pass to snapshot open-position basis. Without
    // carried basis (a single asset has none) the replay starts from the
    // same state and repeats this pass exactly, for lot and pool methods
    // alike, so the snapshot is taken from this cursor.
    let open_basis = cursor.open_position_basis()?.unwrap_or_default();
    let effective_basis = cursor.effective_basis();
    finish(
        &prepared,
        &fragments,
        &effective_basis,
        &open_basis,
        country,
    )
}

/// Derives the `ComputedData` views, in RP2's order.
pub(crate) fn finish(
    prepared: &PreparedAsset,
    fragments: &[Fragment],
    effective_basis: &[Decimal],
    open_basis: &[Decimal],
    country: Country,
) -> EngineResult<AssetOutput> {
    let entries = &prepared.entries;
    let lots = &prepared.lots;
    let values = fragments
        .iter()
        .map(|fragment| fragment_values(fragment, entries, lots, country.long_term_days()))
        .collect::<EngineResult<Vec<_>>>()?;
    let yearly = yearly(&prepared.asset, fragments, &values, entries)?;
    for basis in open_basis {
        if lt13(basis, &zero())? {
            return Err(EngineError::value(format!(
                "Parameter 'open_position_basis' has non-positive value {basis}"
            )));
        }
    }
    let balances = balances(entries, &prepared.ins, &prepared.outs, &prepared.intras)?;
    let sold = sold_percentages(fragments, lots)?;
    let mut gain_losses: Vec<_> = fragments
        .iter()
        .zip(&values)
        .map(|(fragment, value)| gain_loss_out(fragment, value, entries, lots))
        .collect();
    if country == Country::Austria {
        // Kassiber classifies every row of an Austrian book, whatever the
        // method; a failure aborts the report only when Kassiber reaches it.
        for ((out, fragment), value) in gain_losses.iter_mut().zip(fragments).zip(&values) {
            match austria::classify(fragment, &value.fiat_gain, entries, lots) {
                Ok(category) => out.at_category = Some(category.as_str().to_owned()),
                Err(error) => {
                    out.at_category_error = Some(ErrorBody {
                        class: error.class,
                        message: error.message,
                    })
                }
            }
        }
    }
    Ok(AssetOutput {
        asset: prepared.asset.clone(),
        in_transactions: lots.iter().map(|lot| lot.row).collect(),
        in_fiat_in_with_fee: effective_basis.iter().cloned().map(Dec).collect(),
        gain_losses,
        yearly,
        open_positions: open_positions(lots, &sold, open_basis),
        balances,
    })
}
