//! The walk's order (design §4.6): the adapter's `_ordered_rp2_items`
//! (`tests/tax_oracle/adapter.py:656-691`) with the same-time transfer order
//! of `kassiber/core/transfer_chronology.py:48-135`, ported literally. Python
//! keeps its own copy of the latter for custody reconciliation.
//!
//! The items normalize met are bucketed by their row's `occurred_at` text (a
//! move by its out leg's), and the buckets are walked in text order, so two
//! spellings of one instant are two buckets. Within a bucket the inbound
//! events come first, then the moves, then the outbound events, each in the
//! order normalize met them, except that the moves follow their
//! dependencies:
//!
//! - a hard edge runs from a move to one that spends its on-chain
//!   transaction (chain-order evidence on both out legs);
//! - a soft edge runs from a move into a wallet to a move out of that wallet,
//!   unless both moves are slices of one transaction, or the edge would
//!   contradict the hard edges (it joins two moves one already reaches from
//!   the other, or closes a cycle through the edges so far);
//! - the moves are sorted topologically, earliest first among the ready
//!   ones; on a cycle the bucket keeps normalize's order.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::normalize::{Item, Normalized};
use super::request::{ChainOrder, Direction, Row};

/// The order the walk visits one asset's events and moves in.
pub(crate) fn walk_order(rows: &[Row], normalized: &Normalized) -> Vec<Item> {
    let inbound_leg: HashMap<usize, usize> = normalized
        .transfers
        .iter()
        .map(|transfer| (transfer.out, transfer.inbound))
        .collect();
    // `str(occurred_at)` keys; Rust orders `str` by code point, as Python
    // does.
    let mut buckets: BTreeMap<&str, Vec<(usize, Item)>> = BTreeMap::new();
    for (index, &item) in normalized.ordered_items.iter().enumerate() {
        let (Item::Event(row) | Item::Transfer(row)) = item;
        buckets
            .entry(rows[row].ts.raw())
            .or_default()
            .push((index, item));
    }
    let mut ordered = Vec::with_capacity(normalized.ordered_items.len());
    for bucket in buckets.into_values() {
        // Each bucket lists its items in normalize's order already, which
        // is what sorting them by index gives.
        let mut inbound = Vec::new();
        let mut moves = Vec::new();
        let mut outbound = Vec::new();
        for (index, item) in bucket {
            match item {
                Item::Transfer(out) => moves.push(Move {
                    index,
                    out,
                    inbound: inbound_leg[&out],
                }),
                Item::Event(row) if rows[row].direction == Direction::Inbound => inbound.push(item),
                Item::Event(_) => outbound.push(item),
            }
        }
        ordered.extend(inbound);
        ordered.extend(
            same_time_moves(rows, &moves)
                .into_iter()
                .map(|position| Item::Transfer(moves[position].out)),
        );
        ordered.extend(outbound);
    }
    ordered
}

/// A move of one bucket: its position in normalize's order and its legs.
#[derive(Clone, Copy, Debug)]
struct Move {
    index: usize,
    out: usize,
    inbound: usize,
}

/// Every move reachable from `start` along `edges`.
fn descendants(edges: &[BTreeSet<usize>], start: usize) -> HashSet<usize> {
    let mut found = HashSet::new();
    let mut pending: Vec<usize> = edges[start].iter().copied().collect();
    while let Some(target) = pending.pop() {
        if found.insert(target) {
            pending.extend(edges[target].iter().copied());
        }
    }
    found
}

/// Whether `end` is `start` or reachable from it along `edges`.
fn reaches(edges: &[BTreeSet<usize>], start: usize, end: usize) -> bool {
    start == end || descendants(edges, start).contains(&end)
}

/// `order_same_time_transfers`: the moves of one bucket, as positions in
/// `moves`, in dependency order.
fn same_time_moves(rows: &[Row], moves: &[Move]) -> Vec<usize> {
    let count = moves.len();
    let evidence: Vec<Option<&ChainOrder>> = moves
        .iter()
        .map(|item| rows[item.out].chain_order.as_ref())
        .collect();
    let mut outgoing: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
    for (source, source_proof) in evidence.iter().enumerate() {
        let Some(source_proof) = source_proof else {
            continue;
        };
        for (target, target_proof) in evidence.iter().enumerate() {
            if let (true, Some(target_proof)) = (source != target, target_proof) {
                if target_proof.parents.contains(&source_proof.scope) {
                    outgoing[source].insert(target);
                }
            }
        }
    }
    let chain_dependencies = outgoing.iter().any(|targets| !targets.is_empty());
    // The hard edges' closure, fixed before any soft edge is added.
    let hard: Vec<HashSet<usize>> = if chain_dependencies {
        (0..count)
            .map(|item| descendants(&outgoing, item))
            .collect()
    } else {
        Vec::new()
    };
    for source in 0..count {
        for target in 0..count {
            if let (Some(source_proof), Some(target_proof)) = (evidence[source], evidence[target]) {
                if source_proof.scope == target_proof.scope {
                    continue;
                }
            }
            let wallet_hint =
                rows[moves[source].inbound].wallet_id == rows[moves[target].out].wallet_id;
            if source == target || !wallet_hint {
                continue;
            }
            if chain_dependencies {
                if hard[source].contains(&target) || hard[target].contains(&source) {
                    continue;
                }
                // Through every edge so far, soft ones included.
                if reaches(&outgoing, target, source) {
                    continue;
                }
            }
            outgoing[source].insert(target);
        }
    }
    let mut incoming = vec![0_usize; count];
    for targets in &outgoing {
        for &target in targets {
            incoming[target] += 1;
        }
    }
    let position = |item: &usize| moves[*item].index;
    let mut ready: Vec<usize> = (0..count).filter(|&item| incoming[item] == 0).collect();
    ready.sort_by_key(position);
    let mut ordered = Vec::with_capacity(count);
    while !ready.is_empty() {
        let item = ready.remove(0);
        ordered.push(item);
        let mut targets: Vec<usize> = outgoing[item].iter().copied().collect();
        targets.sort_by_key(position);
        for target in targets {
            incoming[target] -= 1;
            if incoming[target] == 0 {
                ready.push(target);
            }
        }
        ready.sort_by_key(position);
    }
    if ordered.len() != count {
        // A cycle: normalize's order.
        return (0..count).collect();
    }
    ordered
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;
    use crate::ledger::normalize::Transfer;
    use crate::ledger::request::LedgerRequest;

    /// A row of the bucket test books: only its time, wallet, direction, and
    /// chain evidence matter here.
    fn row(
        id: &str,
        wallet: &str,
        direction: &str,
        at: &str,
        chain: Option<(&str, &[&str])>,
    ) -> Value {
        let scope = |txid: &str| json!(["bitcoin", "main", txid, "BTC"]);
        json!({
            "id": format!("custody-tax:{id}"),
            "anchor": id,
            "asset": "BTC",
            "wallet_id": wallet,
            "direction": direction,
            "ts": {"raw": at, "error": "not read here"},
            "cutoff_instant": {"error": "not read here"},
            "amount_msat": 1_000,
            "fee_msat": 0,
            "rate": null,
            "value": null,
            "coarse": false,
            "kind": "",
            "text": null,
            "regime_override": null,
            "category_override": null,
            "taxability_off": false,
            "privacy_hop": false,
            "hold": null,
            "chain_order": chain.map(|(txid, parents)| json!({
                "scope": scope(txid),
                "parents": parents.iter().map(|parent| scope(parent)).collect::<Vec<_>>(),
            })),
        })
    }

    fn rows(rows: Vec<Value>) -> Vec<Row> {
        let request: LedgerRequest = serde_json::from_value(json!({
            "schema_version": 2,
            "operation": "prepare",
            "policy": {
                "country": {"kind": "generic", "long_term_days": 365},
                "method": "fifo",
                "method_label": "FIFO",
                "require_coarse_review": false,
            },
            "config": {"assets": ["BTC"], "exchanges": [], "holders": []},
            "holder": null,
            "wallets": [],
            "rows": rows,
            "intra_pairs": [],
            "cross_asset_pairs": [],
        }))
        .expect("a request");
        request.rows
    }

    /// A normalized asset whose items are `items` in that order: `"e<i>"`
    /// is an event at row `i`, `"t<i>:<j>"` a move from row `i` to row `j`.
    fn normalized(items: &[&str]) -> Normalized {
        let mut out = Normalized::default();
        for item in items {
            if let Some(row) = item.strip_prefix('e') {
                out.ordered_items.push(Item::Event(row.parse().unwrap()));
                continue;
            }
            let (from, to) = item[1..].split_once(':').unwrap();
            let (from, to) = (from.parse().unwrap(), to.parse().unwrap());
            out.ordered_items.push(Item::Transfer(from));
            out.transfers.push(Transfer {
                pair: out.transfers.len(),
                out: from,
                inbound: to,
                from_wallet_label: None,
                to_wallet_label: None,
                sent: crate::decimal::Decimal::zero(),
                received: crate::decimal::Decimal::zero(),
                fee: crate::decimal::Decimal::zero(),
                spot_price: None,
                description: String::new(),
                from_cost_basis_pool_id: "global",
                to_cost_basis_pool_id: "global",
                at_regime: None,
                regime_flows: None,
                group_id: None,
            });
        }
        out
    }

    fn names(order: &[Item]) -> Vec<String> {
        order
            .iter()
            .map(|item| match *item {
                Item::Event(row) => format!("e{row}"),
                Item::Transfer(row) => format!("t{row}"),
            })
            .collect()
    }

    const T1: &str = "2024-01-01T00:00:00Z";
    const T2: &str = "2024-01-02T00:00:00Z";

    #[test]
    fn buckets_follow_the_text_of_their_time_and_put_inbounds_first() {
        let rows = rows(vec![
            row("sell", "A", "outbound", T1, None),
            row("late-buy", "A", "inbound", T2, None),
            row("buy", "A", "inbound", T1, None),
            row("move-out", "A", "outbound", T1, None),
            row("move-in", "B", "inbound", T1, None),
            // The same instant spelled another way sorts as its text.
            row("plus", "A", "inbound", "2024-01-01T00:00:00+00:00", None),
        ]);
        let order = walk_order(&rows, &normalized(&["e0", "e1", "e2", "t3:4", "e5"]));
        assert_eq!(names(&order), ["e5", "e2", "t3", "e0", "e1"]);
    }

    #[test]
    fn a_move_into_a_wallet_precedes_a_move_out_of_it() {
        let rows = rows(vec![
            row("b-c-out", "B", "outbound", T1, None),
            row("b-c-in", "C", "inbound", T1, None),
            row("a-b-out", "A", "outbound", T1, None),
            row("a-b-in", "B", "inbound", T1, None),
            row("c-d-out", "C", "outbound", T1, None),
            row("c-d-in", "D", "inbound", T1, None),
        ]);
        let order = walk_order(&rows, &normalized(&["t0:1", "t2:3", "t4:5"]));
        assert_eq!(names(&order), ["t2", "t0", "t4"]);
    }

    #[test]
    fn a_move_made_ready_joins_the_waiting_ones_in_normalizes_order() {
        // The second move funds the first's source; the third has no hint.
        // Once the second is booked, the first goes before the third,
        // which has waited since the start.
        let rows = rows(vec![
            row("b-c-out", "B", "outbound", T1, None),
            row("b-c-in", "C", "inbound", T1, None),
            row("a-b-out", "A", "outbound", T1, None),
            row("a-b-in", "B", "inbound", T1, None),
            row("a-c-out", "A", "outbound", T1, None),
            row("a-c-in", "C", "inbound", T1, None),
        ]);
        let order = walk_order(&rows, &normalized(&["t0:1", "t2:3", "t4:5"]));
        assert_eq!(names(&order), ["t2", "t0", "t4"]);
    }

    #[test]
    fn a_wallet_cycle_keeps_normalizes_order() {
        let rows = rows(vec![
            row("a-b-out", "A", "outbound", T1, None),
            row("a-b-in", "B", "inbound", T1, None),
            row("b-a-out", "B", "outbound", T1, None),
            row("b-a-in", "A", "inbound", T1, None),
            row("x-y-out", "X", "outbound", T1, None),
            row("x-y-in", "Y", "inbound", T1, None),
        ]);
        let order = walk_order(&rows, &normalized(&["t2:3", "t4:5", "t0:1"]));
        assert_eq!(names(&order), ["t2", "t4", "t0"]);
    }

    #[test]
    fn chain_evidence_orders_moves_and_overrules_wallet_hints() {
        // The second move spends the first one's transaction, though its
        // wallets suggest the reverse order.
        let rows = rows(vec![
            row(
                "late-out",
                "B",
                "outbound",
                T1,
                Some(("child", &["parent"])),
            ),
            row("late-in", "A", "inbound", T1, None),
            row("early-out", "A", "outbound", T1, Some(("parent", &[]))),
            row("early-in", "B", "inbound", T1, None),
        ]);
        let order = walk_order(&rows, &normalized(&["t0:1", "t2:3"]));
        assert_eq!(names(&order), ["t2", "t0"]);
    }

    #[test]
    fn slices_of_one_transaction_take_no_wallet_hint() {
        // Both moves are slices of one on-chain transaction, so the hint
        // from wallet B, which would put the second first, is not taken.
        let rows = rows(vec![
            row("first-out", "B", "outbound", T1, Some(("tx", &[]))),
            row("first-in", "C", "inbound", T1, None),
            row("second-out", "A", "outbound", T1, Some(("tx", &[]))),
            row("second-in", "B", "inbound", T1, None),
        ]);
        let order = walk_order(&rows, &normalized(&["t0:1", "t2:3"]));
        assert_eq!(names(&order), ["t0", "t2"]);
        // Without chain evidence the same wallets order the moves.
        let plain = self::rows(vec![
            row("first-out", "B", "outbound", T1, None),
            row("first-in", "C", "inbound", T1, None),
            row("second-out", "A", "outbound", T1, None),
            row("second-in", "B", "inbound", T1, None),
        ]);
        let order = walk_order(&plain, &normalized(&["t0:1", "t2:3"]));
        assert_eq!(names(&order), ["t2", "t0"]);
    }

    #[test]
    fn a_wallet_hint_that_would_close_a_cycle_is_dropped() {
        // Chain: the first move's transaction is spent by the second. The
        // wallets hint the reverse, which the hard edge already orders, so
        // the hint is not taken.
        let rows = rows(vec![
            row("parent-out", "B", "outbound", T1, Some(("parent", &[]))),
            row("parent-in", "C", "inbound", T1, None),
            row(
                "child-out",
                "A",
                "outbound",
                T1,
                Some(("child", &["parent"])),
            ),
            row("child-in", "B", "inbound", T1, None),
        ]);
        let order = walk_order(&rows, &normalized(&["t0:1", "t2:3"]));
        assert_eq!(names(&order), ["t0", "t2"]);
        // A soft edge already added is followed too. Three moves whose
        // wallet hints form a ring, listed against the hints' order, with one
        // chain edge elsewhere so the reachability check runs: the ring's
        // last hint would close a cycle through the two soft edges before
        // it, so it is dropped and the other two order the ring.
        let ring = self::rows(vec![
            row("a-b-out", "A", "outbound", T1, None),
            row("a-b-in", "B", "inbound", T1, None),
            row("c-a-out", "C", "outbound", T1, None),
            row("c-a-in", "A", "inbound", T1, None),
            row("b-c-out", "B", "outbound", T1, None),
            row("b-c-in", "C", "inbound", T1, None),
            row("p-out", "P", "outbound", T1, Some(("p", &[]))),
            row("p-in", "Q", "inbound", T1, None),
            row("q-out", "R", "outbound", T1, Some(("q", &["p"]))),
            row("q-in", "S", "inbound", T1, None),
        ]);
        let order = walk_order(
            &ring,
            &normalized(&["t0:1", "t2:3", "t4:5", "t6:7", "t8:9"]),
        );
        // `order_same_time_transfers` gives `ca, ab, bc, p, q` for these.
        assert_eq!(names(&order), ["t2", "t0", "t4", "t6", "t8"]);
        // Without chain evidence the ring is a cycle, which keeps
        // normalize's order (`ab, ca, bc`).
        let order = walk_order(&ring, &normalized(&["t0:1", "t2:3", "t4:5"]));
        assert_eq!(names(&order), ["t0", "t2", "t4"]);
    }
}
