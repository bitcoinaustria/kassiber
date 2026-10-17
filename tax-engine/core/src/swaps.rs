//! Austrian crypto-to-crypto swaps: pair validation (`validate`) and the
//! multi-asset runner (`compute_multi`).
//!
//! A Neuvermögen swap realizes no gain on its outgoing leg and carries the
//! source pool's basis into the incoming lot on another asset. The runner
//! interleaves the per-asset cursors so each incoming lot receives its carry
//! before any event that could pull it into a pool. This ports the
//! owner-authored `collect_at_swap_link_pairs` (`at.py`) and
//! `at_native_tax_engine.py`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::austria::{
    lot_regime, pool_id, py_repr, regime_from_notes, validated_swap_link_id, Regime,
};
use crate::cursor::{Cursor, EventComputation, Fragment};
use crate::engine::{finish, Country, ParsedAsset, PreparedAsset};
use crate::entry::Entry;
use crate::error::{EngineError, EngineResult};
use crate::methods::lot_method;
use crate::model::{AssetInput, AssetOutput, EntryKind, MethodName};
use crate::num::mul;

/// One validated `at_swap_link=<id>` pair: an outgoing leg on one asset and
/// an incoming leg on another, as (asset position, entry index).
#[derive(Clone, Debug)]
struct SwapPair {
    id: String,
    out_asset: usize,
    out_entry: usize,
    in_asset: usize,
    in_entry: usize,
}

/// Parses every asset in request order; the first constructor failure
/// wins, as it does while the adapter builds its transaction sets.
fn parse_assets(inputs: &[AssetInput]) -> EngineResult<Vec<ParsedAsset>> {
    let mut names = HashSet::new();
    for input in inputs {
        if !names.insert(input.asset.as_str()) {
            return Err(EngineError::request(format!(
                "asset {:?} appears more than once",
                input.asset
            )));
        }
    }
    inputs.iter().map(ParsedAsset::new).collect()
}

/// `(asset, internal_id)` tuples as Python prints them.
fn legs_repr(assets: &[ParsedAsset], legs: &[(usize, usize)]) -> String {
    let parts: Vec<String> = legs
        .iter()
        .map(|&(asset, entry)| {
            format!(
                "({}, {})",
                py_repr(&assets[asset].asset),
                py_repr(&assets[asset].entries[entry].internal_id())
            )
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

/// `collect_at_swap_link_pairs`: every marked, swap-eligible OUT and IN
/// entry (Intra entries are not scanned), checked id by id in sorted order.
fn collect_pairs(assets: &[ParsedAsset]) -> EngineResult<Vec<SwapPair>> {
    let mut outs: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    let mut ins: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (position, asset) in assets.iter().enumerate() {
        for (kind, legs) in [(EntryKind::Out, &mut outs), (EntryKind::In, &mut ins)] {
            for entry in asset.visible(kind) {
                if let Some(id) = validated_swap_link_id(&asset.entries[entry])? {
                    legs.entry(id).or_default().push((position, entry));
                }
            }
        }
    }
    let ids: BTreeSet<&String> = outs.keys().chain(ins.keys()).collect();
    let mut pairs = Vec::with_capacity(ids.len());
    for id in ids {
        let out_legs = outs.get(id).map_or(&[][..], Vec::as_slice);
        let in_legs = ins.get(id).map_or(&[][..], Vec::as_slice);
        let (&[(out_asset, out_entry)], &[(in_asset, in_entry)]) = (out_legs, in_legs) else {
            return Err(EngineError::value(format!(
                "Unpaired `at_swap_link={id}` marker: expected exactly one OutTransaction and one InTransaction, found {} outgoing and {} incoming. Kassiber must emit both legs of every crypto-to-crypto swap before RP2 can honor the marker. Outgoing: {}; Incoming: {}",
                out_legs.len(),
                in_legs.len(),
                legs_repr(assets, out_legs),
                legs_repr(assets, in_legs)
            )));
        };
        let out_name = &assets[out_asset].asset;
        let in_name = &assets[in_asset].asset;
        let outgoing = &assets[out_asset].entries[out_entry];
        let incoming = &assets[in_asset].entries[in_entry];
        if out_name == in_name {
            return Err(EngineError::value(format!(
                "`at_swap_link={id}` pair is same-asset (both legs on {out_name}). A crypto-to-crypto swap crosses two assets; a same-asset pair indicates a Kassiber emission bug or a misclassified same-asset transfer. Outgoing: {}; Incoming: {}",
                outgoing.row, incoming.row
            )));
        }
        if incoming.ts.us < outgoing.ts.us {
            return Err(EngineError::value(format!(
                "`at_swap_link={id}` incoming leg is earlier than the outgoing leg. RP2 cannot carry basis backwards in time. Outgoing: {out_name} {}; Incoming: {in_name} {}",
                outgoing.row, incoming.row
            )));
        }
        pairs.push(SwapPair {
            id: id.clone(),
            out_asset,
            out_entry,
            in_asset,
            in_entry,
        });
    }
    Ok(pairs)
}

/// `validate_input_data`: Austria checks the swap pairs; other countries
/// have nothing to check.
pub(crate) fn validate(inputs: &[AssetInput], country: Country) -> EngineResult<()> {
    let assets = parse_assets(inputs)?;
    if country == Country::Austria {
        collect_pairs(&assets)?;
    }
    Ok(())
}

/// `compute_tax_for_assets`: the per-asset results of the Austrian runner,
/// or `None` when there is no swap pair (or the country is not Austria) and
/// the caller computes each asset on its own.
pub(crate) fn compute_multi(
    inputs: &[AssetInput],
    country: Country,
    method: MethodName,
) -> EngineResult<Option<Vec<AssetOutput>>> {
    let assets = parse_assets(inputs)?;
    if country != Country::Austria {
        return Ok(None);
    }
    let pairs = collect_pairs(&assets)?;
    if pairs.is_empty() {
        return Ok(None);
    }
    run(assets, pairs, method).map(Some)
}

/// `_incoming_can_affect_event`: whether an unresolved incoming lot could
/// enter the pool `event` consumes. An unmarked event counts as Neu.
fn can_affect(incoming: &Entry, event: &Entry) -> EngineResult<bool> {
    if lot_regime(incoming)? != Regime::Neu {
        return Ok(false);
    }
    if regime_from_notes(&event.notes)? == Some(Regime::Alt) {
        return Ok(false);
    }
    Ok(pool_id(&incoming.notes)? == pool_id(&event.notes)?)
}

/// `_event_sort_key`: instant, then row.
fn event_order(a: &Entry, b: &Entry) -> Ordering {
    a.ts.us.cmp(&b.ts.us).then(a.row.cmp(&b.row))
}

/// `_find_dependency_cycle`: an iterative depth-first search over the
/// nodes in sorted order, visiting neighbours in sorted order; the first
/// back edge closes the reported cycle.
fn find_cycle<'a>(graph: &BTreeMap<&'a str, BTreeSet<&'a str>>) -> Option<Vec<&'a str>> {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut active: HashSet<&str> = HashSet::new();
    // Each frame: a node and the position of its next neighbour.
    let mut stack: Vec<(&str, Vec<&str>, usize)> = Vec::new();
    let neighbours = |node: &str| -> Vec<&'a str> {
        graph
            .get(node)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    };
    for &start in graph.keys() {
        if visited.contains(start) {
            continue;
        }
        active.insert(start);
        stack.push((start, neighbours(start), 0));
        while let Some((current, next, position)) = stack.last_mut() {
            let current = *current;
            let Some(&dependency) = next.get(*position) else {
                stack.pop();
                active.remove(current);
                visited.insert(current);
                continue;
            };
            *position += 1;
            if active.contains(dependency) {
                let from = stack
                    .iter()
                    .position(|(node, _, _)| *node == dependency)
                    .unwrap_or(0);
                let mut cycle: Vec<&str> = stack[from..].iter().map(|(node, _, _)| *node).collect();
                cycle.push(dependency);
                return Some(cycle);
            }
            if visited.contains(dependency) {
                continue;
            }
            active.insert(dependency);
            stack.push((dependency, neighbours(dependency), 0));
        }
    }
    None
}

/// `_reject_cyclic_swap_dependencies`: an outgoing leg depends on every
/// other pair whose incoming leg is on its asset, no later than it, and
/// could affect it. A cycle can never be ordered.
fn reject_cycles(
    assets: &[ParsedAsset],
    pairs: &[SwapPair],
    incoming: &[Vec<usize>],
) -> EngineResult<()> {
    let entry = |asset: usize, index: usize| &assets[asset].entries[index];
    let mut graph: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for pair in pairs {
        let outgoing = entry(pair.out_asset, pair.out_entry);
        let mut dependencies = BTreeSet::new();
        for &blocker in &incoming[pair.out_asset] {
            let blocker = &pairs[blocker];
            if blocker.id == pair.id {
                continue;
            }
            let leg = entry(blocker.in_asset, blocker.in_entry);
            if leg.ts.us > outgoing.ts.us {
                continue;
            }
            if can_affect(leg, outgoing)? {
                dependencies.insert(blocker.id.as_str());
            }
        }
        graph.insert(pair.id.as_str(), dependencies);
    }
    match find_cycle(&graph) {
        None => Ok(()),
        Some(cycle) => {
            let summary: Vec<String> = cycle
                .iter()
                .map(|id| format!("at_swap_link={id}"))
                .collect();
            Err(EngineError::value(format!(
                "Cyclic Austrian swap basis dependency: same-pool Neu swap legs cannot carry basis before their own incoming leg has been resolved ({}). Add distinct at_pool markers for independent pools, or split the transactions into an order that does not require using unresolved carried basis.",
                summary.join(" -> ")
            )))
        }
    }
}

/// The runner's view of the pairs and assets while cursors advance.
struct Runner<'a> {
    prepared: &'a [PreparedAsset],
    pairs: &'a [SwapPair],
    /// Per destination asset, its pairs ordered by incoming leg then id.
    incoming: &'a [Vec<usize>],
    resolved: Vec<bool>,
}

impl Runner<'_> {
    fn entry(&self, asset: usize, index: usize) -> &Entry {
        &self.prepared[asset].entries[index]
    }

    /// `_first_unresolved_incoming_pair`: the first pair, in incoming
    /// order and no later than `event`, that is unresolved and could affect
    /// it.
    fn blocker(&self, asset: usize, event: &Entry) -> EngineResult<Option<usize>> {
        for &pair in &self.incoming[asset] {
            let leg = self.entry(self.pairs[pair].in_asset, self.pairs[pair].in_entry);
            if leg.ts.us > event.ts.us {
                return Ok(None);
            }
            if !self.resolved[pair] && can_affect(leg, event)? {
                return Ok(Some(pair));
            }
        }
        Ok(None)
    }

    /// `_consumed_neu_lot`: whether any fragment's lot is Neuvermögen.
    fn consumed_neu(&self, asset: usize, fragments: &[Fragment]) -> EngineResult<bool> {
        let prepared = &self.prepared[asset];
        for fragment in fragments {
            if let Some(lot) = fragment.lot {
                if lot_regime(&prepared.entries[prepared.lots[lot].entry])? == Regime::Neu {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

/// `compute_native_at_tax`.
fn run(
    assets: Vec<ParsedAsset>,
    pairs: Vec<SwapPair>,
    method: MethodName,
) -> EngineResult<Vec<AssetOutput>> {
    let source: HashMap<(usize, i64), usize> = pairs
        .iter()
        .enumerate()
        .map(|(index, pair)| {
            let row = assets[pair.out_asset].entries[pair.out_entry].row;
            ((pair.out_asset, row), index)
        })
        .collect();
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); assets.len()];
    for (index, pair) in pairs.iter().enumerate() {
        incoming[pair.in_asset].push(index);
    }
    for list in &mut incoming {
        list.sort_by(|&a, &b| {
            let leg = |pair: usize| &assets[pairs[pair].in_asset].entries[pairs[pair].in_entry];
            event_order(leg(a), leg(b)).then_with(|| pairs[a].id.cmp(&pairs[b].id))
        });
    }
    reject_cycles(&assets, &pairs, &incoming)?;

    let prepared = assets
        .into_iter()
        .map(PreparedAsset::from_parsed)
        .collect::<EngineResult<Vec<_>>>()?;
    // The destination lot of each pair, when the incoming leg is a visible
    // lot of its asset.
    let destination: Vec<Option<usize>> = pairs
        .iter()
        .map(|pair| {
            prepared[pair.in_asset]
                .lots
                .iter()
                .position(|lot| lot.entry == pair.in_entry)
        })
        .collect();
    let mut cursors: Vec<Cursor<'_>> = prepared
        .iter()
        .map(|asset| {
            Cursor::new(
                &asset.entries,
                &asset.lots,
                &asset.events,
                lot_method(method, &asset.entries, &asset.lots),
            )
        })
        .collect();
    let mut fragments: Vec<Vec<Fragment>> = vec![Vec::new(); prepared.len()];
    let mut runner = Runner {
        prepared: &prepared,
        pairs: &pairs,
        incoming: &incoming,
        resolved: vec![false; pairs.len()],
    };

    loop {
        let mut current: Vec<(usize, &Entry)> = cursors
            .iter()
            .enumerate()
            .filter_map(|(asset, cursor)| cursor.peek().map(|event| (asset, event)))
            .collect();
        if current.is_empty() {
            break;
        }
        current.sort_by(|(a, x), (b, y)| {
            event_order(x, y).then_with(|| prepared[*a].asset.cmp(&prepared[*b].asset))
        });
        let mut progressed = false;
        for &(asset, event) in &current {
            if runner.blocker(asset, event)?.is_some() {
                continue;
            }
            let Some(result) = cursors[asset].consume_next(&mut fragments[asset])? else {
                return Err(EngineError::runtime(
                    "Internal error: a cursor had no taxable event to consume",
                ));
            };
            if let Some(&pair) = source.get(&(asset, event.row)) {
                let produced = fragments[asset].len() - result.fragments;
                if result.taxable_event_unit_cost_basis.is_none()
                    && runner.consumed_neu(asset, &fragments[asset][produced..])?
                {
                    let pair = &pairs[pair];
                    return Err(EngineError::value(format!(
                        "Austrian swap neutrality requires the `moving_average_at` accounting method, but the Neuvermögen outgoing leg of at_swap_link={} ({} {}) produced no carried cost basis. The configured accounting method did not honor the swap marker. Re-run with `-m moving_average_at` (the AT default), or remove the at_swap_link markers if you intend swaps to be taxable disposals.",
                        pair.id, prepared[pair.out_asset].asset, event.row
                    )));
                }
                resolve(
                    &mut runner,
                    &mut cursors,
                    &destination,
                    pair,
                    event,
                    &result,
                )?;
            }
            progressed = true;
            break;
        }
        if !progressed {
            let mut waits = Vec::new();
            for &(asset, event) in &current {
                if let Some(pair) = runner.blocker(asset, event)? {
                    let pair = &pairs[pair];
                    waits.push(format!(
                        "{} event {} at {} waits for at_swap_link={} from {} event {}",
                        prepared[asset].asset,
                        event.row,
                        event.ts.iso,
                        pair.id,
                        prepared[pair.out_asset].asset,
                        runner.entry(pair.out_asset, pair.out_entry).row
                    ));
                }
            }
            return Err(EngineError::value(format!(
                "Unable to order Austrian swap-linked taxable events without using unresolved carried basis: {}",
                waits.join("; ")
            )));
        }
    }

    // `to_computed_data`, asset by asset: a replay with the final carried
    // basis snapshots the open positions; the gain/loss rows come from the
    // interleaved pass.
    let mut outputs = Vec::with_capacity(prepared.len());
    for ((asset, cursor), fragments) in prepared.iter().zip(cursors.iter_mut()).zip(&fragments) {
        let overrides = cursor.state_mut().basis_override.clone();
        let mut replay = Cursor::new(
            &asset.entries,
            &asset.lots,
            &asset.events,
            lot_method(method, &asset.entries, &asset.lots),
        );
        replay.state_mut().basis_override = overrides;
        replay.run(&mut Vec::new())?;
        let open_basis = replay.open_position_basis()?.unwrap_or_default();
        let effective_basis = cursor.effective_basis();
        outputs.push(finish(
            asset,
            fragments,
            &effective_basis,
            &open_basis,
            Country::Austria,
        )?);
    }
    Ok(outputs)
}

/// `_resolve_swap_pair`: writes the carried basis `crypto_taxable_amount ×
/// carried unit basis` onto the destination lot, then marks the pair
/// resolved (also when nothing is carried).
fn resolve(
    runner: &mut Runner<'_>,
    cursors: &mut [Cursor<'_>],
    destination: &[Option<usize>],
    pair: usize,
    event: &Entry,
    result: &EventComputation,
) -> EngineResult<()> {
    if runner.resolved[pair] {
        return Err(EngineError::runtime(format!(
            "Internal error: swap pair {} resolved more than once",
            runner.pairs[pair].id
        )));
    }
    if let Some(unit) = &result.taxable_event_unit_cost_basis {
        let carried = mul(&event.crypto_taxable_amount(), unit)?;
        if let Some(lot) = destination[pair] {
            cursors[runner.pairs[pair].in_asset]
                .state_mut()
                .basis_override[lot] = Some(carried);
        }
    }
    runner.resolved[pair] = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_follow_sorted_depth_first_order() {
        let mut graph: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        graph.insert("aa", ["mm"].into_iter().collect());
        graph.insert("mm", ["zz"].into_iter().collect());
        graph.insert("zz", ["aa"].into_iter().collect());
        assert_eq!(find_cycle(&graph), Some(vec!["aa", "mm", "zz", "aa"]));
        let mut chain: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        chain.insert("a", ["b"].into_iter().collect());
        chain.insert("b", BTreeSet::new());
        assert_eq!(find_cycle(&chain), None);
    }
}
