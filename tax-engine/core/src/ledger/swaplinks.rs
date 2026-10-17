//! Austrian swap-link selection (design §4.9): the adapter's
//! `_select_at_cross_asset_swap_links` (`tests/tax_oracle/adapter.py:1819-1889`)
//! with `_prepared_quarantine_reasons` (`:1729-1743`), `_carry_pair_leg_reason`
//! and `_carry_pair_reason` (`:1746-1785`), `_swap_pair_quarantines`
//! (`:1788-1816`), and `_carry_pair_blocked_row_ids` (`:2004-2017`), ported
//! literally. The generic rail carry ([`super::carry`]) shares the reason
//! lookup, the pair key, and the withheld rows, as the adapter's carry does.
//!
//! It runs on an Austrian book after the first prepare pass, over the
//! book's carrying-value cross-asset pairs in request order, and reads only
//! that pass: its normalized events (with their regimes) and the first
//! reason its quarantines, normalize's then the gates', give each
//! transaction. A pair with a leg that is not a row is skipped, so its rows
//! book as an ordinary disposal and acquisition. Otherwise:
//!
//! - a pair a leg of which normalize did not make an event, or a leg of
//!   which the first pass quarantined (looked up by the row's id, its
//!   transaction, then the pair's transaction id for that side, out leg
//!   first), is blocked: both legs (only the out leg for a direct payout)
//!   are quarantined with that reason, else `swap_leg_unavailable`, and its
//!   rows, with a direct payout's target-asset disposal `<pair>:out`, are
//!   left out of the second pass;
//! - a pair whose out leg's first-pass regime is not Neu is skipped (an Alt
//!   swap books as a disposal and an acquisition at market);
//! - every other pair links its two rows by its id, else `<out>-><in>`.
//!
//! The second pass does not check a link again: its regimes may differ, and
//! an asset the blocked rows leave without an inbound entry drops its swap
//! legs, which can leave a link unpaired for the lot engine's check (design
//! §4.9). Both are the adapter's behaviour.

use std::collections::{BTreeSet, HashMap};

use serde_json::{json, Map, Value};

use crate::error::EngineResult;
use crate::manifest::{DetailKind, QuarantineReason, SWAP_QUARANTINE_REASON};
use crate::num;

use super::normalize::{btc, Event, Normalized, Quarantine};
use super::request::{Country, CrossAssetPair, Regime, Row};
use super::run::Book;
use super::walk::Walked;

/// The cross-asset pair policy whose pairs carry basis (the swap carry's
/// and the generic rail carry's).
pub(crate) const CARRYING_VALUE: &str = "carrying-value";

/// The prefix of a direct payout's pair id; such a pair also withholds its
/// target-asset disposal, `<pair id>:out`.
const DIRECT_PAYOUT: &str = "direct-payout:";

/// The reason code of a blocked pair a leg of which normalize dropped
/// without a quarantine naming its transaction.
pub(crate) const SWAP_LEG_UNAVAILABLE: &str = "swap_leg_unavailable";

/// What the selection found after the first pass.
#[derive(Clone, Debug, Default)]
pub(crate) struct Selection {
    /// Each linked row and its link, in the order the pairs linked them
    /// (a row a later pair links again keeps its place).
    pub links: Vec<(usize, String)>,
    /// The ids of the rows the blocked pairs withhold from the second pass.
    /// A direct payout's `<pair>:out` is among them whether or not a row has
    /// that id.
    pub blocked: BTreeSet<String>,
    /// The blocked pairs' quarantines, in pair order.
    pub quarantines: Vec<Quarantine>,
}

impl Selection {
    /// Whether the book needs a second pass.
    pub fn changes_the_pass(&self) -> bool {
        !self.links.is_empty() || !self.blocked.is_empty()
    }

    /// `swap_link_by_row_id[row] = link`.
    fn link(&mut self, row: usize, link: &str) {
        match self.links.iter_mut().find(|(linked, _)| *linked == row) {
            Some(entry) => entry.1 = link.to_owned(),
            None => self.links.push((row, link.to_owned())),
        }
    }
}

/// A pair's id, else `<out>-><in>` (`str(pair.get("pair_id") or ...)`).
pub(crate) fn link_id(pair: &CrossAssetPair) -> String {
    match pair.pair_id.as_deref() {
        Some(id) if !id.is_empty() => id.to_owned(),
        _ => format!("{}->{}", pair.out_id, pair.in_id),
    }
}

/// `value or fallback` for an optional text.
pub(crate) fn or_else<'a>(value: Option<&'a str>, fallback: &'a str) -> &'a str {
    match value {
        Some(value) if !value.is_empty() => value,
        _ => fallback,
    }
}

/// The first reason each transaction's quarantines give in a prepare pass
/// (`_prepared_quarantine_reasons`): normalize's quarantines before the
/// gates', asset by asset. The rail carry reads it too.
pub(crate) struct LegReasons<'b> {
    rows: &'b [Row],
    reasons: HashMap<&'b str, QuarantineReason>,
}

impl<'b> LegReasons<'b> {
    pub fn new(rows: &'b [Row]) -> Self {
        LegReasons {
            rows,
            reasons: HashMap::new(),
        }
    }

    /// Adds one asset's stages.
    pub fn add(&mut self, normalized: &Normalized, walked: &Walked) {
        let rows = self.rows;
        for quarantine in normalized.quarantines.iter().chain(&walked.quarantines) {
            self.reasons
                .entry(rows[quarantine.row].anchor.as_str())
                .or_insert(quarantine.reason);
        }
    }

    /// `_carry_pair_leg_reason`: the reason of the row's id, its
    /// transaction, then the pair's transaction id for the side, whichever
    /// is found first.
    pub fn leg(&self, row: usize, pair_transaction: Option<&str>) -> Option<QuarantineReason> {
        let row = &self.rows[row];
        [
            Some(row.id.as_str()),
            Some(row.anchor.as_str()).filter(|anchor| !anchor.is_empty()),
            pair_transaction.filter(|transaction| !transaction.is_empty()),
        ]
        .into_iter()
        .flatten()
        .find_map(|candidate| self.reasons.get(candidate).copied())
    }
}

/// The selection over the first pass's stages, each asset's normalized
/// stage with its walk (every asset was walked, or the pass failed before
/// the selection runs).
pub(crate) fn select<'s>(
    book: &Book,
    first_pass: impl IntoIterator<Item = (&'s Normalized, &'s Walked)>,
) -> EngineResult<Selection> {
    let mut selection = Selection::default();
    let pairs = &book.request.cross_asset_pairs;
    if !matches!(book.request.policy.country, Country::At {}) || pairs.is_empty() {
        return Ok(selection);
    }
    let mut events: HashMap<usize, &Event> = HashMap::new();
    let mut reasons = LegReasons::new(book.rows);
    for (normalized, walked) in first_pass {
        for event in &normalized.events {
            events.insert(event.row, event);
        }
        reasons.add(normalized, walked);
    }
    for pair in pairs {
        if pair.policy.as_deref() != Some(CARRYING_VALUE) {
            continue;
        }
        let (Some(out), Some(inbound)) =
            (book.row_index(&pair.out_id), book.row_index(&pair.in_id))
        else {
            continue;
        };
        let reason = reasons
            .leg(out, pair.out_transaction_id.as_deref())
            .or_else(|| reasons.leg(inbound, pair.in_transaction_id.as_deref()));
        let out_event = events.get(&out);
        // The second pass leaves the blocked rows out, so their first-pass
        // quarantines are not raised again: the pair's quarantines are the
        // only record of the block.
        let blocked_for = if out_event.is_none() || !events.contains_key(&inbound) {
            Some(reason.map_or(SWAP_LEG_UNAVAILABLE, QuarantineReason::as_str))
        } else {
            reason.map(QuarantineReason::as_str)
        };
        if let Some(reason_code) = blocked_for {
            selection
                .quarantines
                .extend(pair_quarantines(book, pair, out, inbound, reason_code)?);
            selection.blocked.extend(blocked_rows(pair));
            continue;
        }
        if out_event.and_then(|event| event.at_regime) != Some(Regime::Neu) {
            continue;
        }
        let link = link_id(pair);
        selection.link(out, &link);
        selection.link(inbound, &link);
    }
    Ok(selection)
}

/// `_swap_pair_quarantines`: both legs share one detail; a direct payout's
/// two settlement legs anchor to one source transaction, so only its out
/// leg is quarantined, without the out amount.
fn pair_quarantines(
    book: &Book,
    pair: &CrossAssetPair,
    out: usize,
    inbound: usize,
    reason_code: &str,
) -> EngineResult<Vec<Quarantine>> {
    let rows = book.rows;
    let link = link_id(pair);
    let mut detail: Map<String, Value> = [
        (
            "outgoing_asset",
            Value::from(or_else(pair.out_asset.as_deref(), &rows[out].asset)),
        ),
        (
            "incoming_asset",
            Value::from(or_else(pair.in_asset.as_deref(), &rows[inbound].asset)),
        ),
        ("at_swap_link", Value::from(link.as_str())),
        ("reason_code", Value::from(reason_code)),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect();
    let quarantine = |row: usize, detail: &Map<String, Value>| Quarantine {
        row,
        reason: SWAP_QUARANTINE_REASON,
        detail_kind: DetailKind::Plain,
        detail: detail.clone(),
    };
    if link.starts_with(DIRECT_PAYOUT) {
        return Ok(vec![quarantine(out, &detail)]);
    }
    // `float(msat_to_btc(amount) or Decimal("0"))`.
    let amount = btc(rows[out].amount_msat.into())?;
    let amount = if num::truthy(&amount) {
        amount
    } else {
        num::zero()
    };
    detail.insert(
        String::from("out_amount"),
        json!({ "float": amount.to_canonical() }),
    );
    Ok(vec![quarantine(out, &detail), quarantine(inbound, &detail)])
}

/// `_carry_pair_blocked_row_ids`: the pair's rows, and for a direct payout
/// (by its own pair id, not the fallback) its target-asset disposal.
pub(crate) fn blocked_rows(pair: &CrossAssetPair) -> Vec<String> {
    let mut blocked = vec![pair.out_id.clone(), pair.in_id.clone()];
    let pair_id = pair.pair_id.as_deref().unwrap_or("");
    if pair_id.starts_with(DIRECT_PAYOUT) {
        blocked.push(format!("{pair_id}:out"));
    }
    blocked
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::ledger::request::LedgerRequest;
    use crate::ledger::run::{ledger, prepare};
    use crate::time::CivilDate;

    // Small Austrian books through the whole pipeline. Every expectation is
    // what the oracle gives for the same rows (projections built with
    // `tests/tax_ledger_generator.py`'s `projected` and `direct`, compared
    // with `tests/tax_engine_compare.py`: production equals the oracle on
    // each); `tests/test_tax_ledger_prepare.py` compares the selection and
    // both passes with the oracle's on the goldens, seeded books, and edge
    // books like these.

    /// 1 USDT in msat.
    const USDT: u64 = 100_000_000_000;

    fn ts(date: &str) -> Value {
        let days = CivilDate::parse(date).expect("a date").to_days();
        json!({
            "raw": format!("{date}T00:00:00Z"),
            "us": days * 86_400 * 1_000_000,
            "offset_s": 0,
            "date": date,
            "year": date[..4].parse::<i32>().expect("a year"),
            "vienna_date": date,
            "display": format!("{date} 00:00:00+00:00"),
            "iso": format!("{date}T00:00:00+00:00"),
        })
    }

    /// A row named `name` (its id `custody-tax:<name>`, its transaction and
    /// text `name`), priced at 40000 unless `fields` says otherwise.
    fn row(
        name: &str,
        wallet: &str,
        direction: &str,
        date: &str,
        amount: u64,
        fields: Value,
    ) -> Value {
        let mut row = json!({
            "id": format!("custody-tax:{name}"),
            "anchor": name,
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
            "text": name,
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

    /// A carrying-value pair between two rows, as the builder sends one.
    fn cross(out: &Value, inbound: &Value, pair_id: Option<&str>, fields: Value) -> Value {
        let mut pair = json!({
            "pair_id": pair_id,
            "out_id": out["id"],
            "in_id": inbound["id"],
            "out_transaction_id": out["anchor"],
            "in_transaction_id": inbound["anchor"],
            "out_asset": out["asset"],
            "in_asset": inbound["asset"],
            "policy": "carrying-value",
            "bitcoin_rail": false,
        });
        for (key, value) in fields.as_object().expect("fields") {
            pair[key] = value.clone();
        }
        pair
    }

    fn request(rows: Vec<Value>, pairs: Vec<Value>, cross: Vec<Value>) -> LedgerRequest {
        serde_json::from_value(json!({
            "schema_version": 2,
            "operation": "ledger",
            "policy": {
                "country": {"kind": "at"},
                "method": "moving_average_at",
                "method_label": "moving_average_at",
                "require_coarse_review": false,
            },
            "config": {
                "assets": ["BTC", "LBTC", "USDT"],
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
            "cross_asset_pairs": cross,
        }))
        .expect("a request")
    }

    fn run(request: &LedgerRequest) -> Value {
        match ledger(request) {
            Ok(response) => serde_json::to_value(response).expect("serializes"),
            Err(error) => json!({"error": serde_json::to_value(error).expect("serializes")}),
        }
    }

    /// The selection and the final pass, as the `prepare` operation lists
    /// them.
    fn prepared(request: &LedgerRequest) -> Value {
        serde_json::to_value(prepare(request).expect("prepares")).expect("serializes")
    }

    /// A purchase of 1 BTC, a 0.5 BTC swap with a fee into 30000 USDT, and
    /// a later USDT sale; `buy` and `swap` move the purchase and the swap.
    fn swap_rows(buy: (&str, u64), swap: &str, swap_in: Value) -> Vec<Value> {
        vec![
            row("buy", "A", "inbound", buy.0, buy.1, json!({})),
            row(
                "swap-out",
                "A",
                "outbound",
                swap,
                50_000_000_000,
                json!({"fee_msat": 10_000_000, "rate": "60000E0"}),
            ),
            row("swap-in", "A", "inbound", swap, 30_000 * USDT, {
                let mut fields = json!({"asset": "USDT", "rate": "1E0"});
                for (key, value) in swap_in.as_object().expect("fields") {
                    fields[key] = value.clone();
                }
                fields
            }),
            row(
                "sell-usdt",
                "A",
                "outbound",
                "2024-09-01",
                10_000 * USDT,
                json!({"asset": "USDT", "rate": "11E-1"}),
            ),
        ]
    }

    fn book(buy: (&str, u64), swap: &str, swap_in: Value) -> (Vec<Value>, Vec<Value>) {
        let rows = swap_rows(buy, swap, swap_in);
        let pair = cross(&rows[1], &rows[2], Some("swap-1"), json!({}));
        (rows, vec![pair])
    }

    /// Each quarantine as `[row name, reason, detail]`.
    fn quarantines(response: &Value) -> Vec<Value> {
        response["quarantines"]
            .as_array()
            .expect("quarantines")
            .iter()
            .map(|item| {
                json!([
                    item["row"]
                        .as_str()
                        .unwrap()
                        .trim_start_matches("custody-tax:"),
                    item["reason"],
                    item["detail"],
                ])
            })
            .collect()
    }

    /// Each entry as `[type, row name, description]`; transfer legs by type.
    fn entries(response: &Value) -> Vec<Value> {
        response["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| match entry["row"].as_str() {
                Some(row) => json!([
                    entry["entry_type"],
                    row.trim_start_matches("custody-tax:"),
                    entry["description"]
                ]),
                None => json!([entry["entry_type"]]),
            })
            .collect()
    }

    fn swap_detail(reason_code: &str, out_amount: Option<&str>) -> Value {
        let mut detail = json!({
            "outgoing_asset": "BTC",
            "incoming_asset": "USDT",
            "at_swap_link": "swap-1",
            "reason_code": reason_code,
        });
        if let Some(amount) = out_amount {
            detail["out_amount"] = json!({ "float": amount });
        }
        detail
    }

    #[test]
    fn a_neu_swap_links_both_legs_and_carries_the_pool_basis() {
        let (rows, pairs) = book(("2024-01-01", 100_000_000_000), "2024-06-01", json!({}));
        let response = run(&request(rows, vec![], pairs));
        assert_eq!(quarantines(&response), Vec::<Value>::new());
        let entries = &response["entries"];
        assert_eq!(
            entries[1]["description"],
            "at_regime=neu at_pool=default at_swap_link=swap-1 swap-out"
        );
        // Zero gain at the swap's proceeds, and the swapped-in USDT takes
        // the pool's basis of what was swapped, its fee aside (0.5 BTC at
        // the pool's 40000).
        assert_eq!(entries[1]["at_category"], "neu_swap");
        assert_eq!(entries[1]["gain_loss"], "0E-27");
        assert_eq!(entries[2]["row"], "custody-tax:swap-in");
        assert_eq!(
            entries[2]["description"],
            "at_regime=neu at_pool=default at_swap_link=swap-1 swap-in"
        );
        assert_eq!(entries[2]["fiat_value"], "200000E-1");
        assert_eq!(entries[3]["at_category"], "neu_gain");
    }

    #[test]
    fn an_alt_swap_is_not_linked() {
        let (rows, pairs) = book(("2020-01-01", 100_000_000_000), "2020-06-01", json!({}));
        let response = run(&request(rows, vec![], pairs));
        assert_eq!(quarantines(&response), Vec::<Value>::new());
        for entry in response["entries"].as_array().unwrap() {
            let text = entry["description"].as_str().unwrap();
            assert!(!text.contains("at_swap_link="), "{text}");
        }
        // An ordinary disposal, and the USDT acquired at its market value.
        assert_eq!(response["entries"][1]["at_category"], "alt_spekulation");
        assert_eq!(response["entries"][2]["fiat_value"], "30000E0");
    }

    #[test]
    fn a_short_swap_out_blocks_its_pair_with_the_gates_reason() {
        let (mut rows, pairs) = book(("2024-01-01", 40_000_000_000), "2024-06-01", json!({}));
        rows.push(row(
            "late-buy",
            "A",
            "inbound",
            "2024-10-01",
            10_000_000_000,
            json!({"rate": null}),
        ));
        let response = run(&request(rows, vec![], pairs));
        // The pair's quarantines come first and keep the first pass's
        // reason, which the second pass, without the pair's rows, does not
        // raise again; the assets' own follow.
        let detail = swap_detail("insufficient_lots", Some("5E-1"));
        assert_eq!(
            quarantines(&response),
            vec![
                json!(["swap-out", "at_swap_basis_carry_unresolved", detail]),
                json!(["swap-in", "at_swap_basis_carry_unresolved", detail]),
                json!([
                    "late-buy",
                    "missing_spot_price",
                    {"asset": "BTC", "direction": "inbound", "required_for": "acquisition", "wallet": "Cold"}
                ]),
                json!([
                    "sell-usdt",
                    "insufficient_lots",
                    {"asset": "USDT", "available": {"float": "0E0"}, "required": {"float": "10000E0"}, "wallet": "Cold"}
                ]),
            ]
        );
        assert_eq!(
            entries(&response),
            vec![json!([
                "acquisition",
                "buy",
                "at_regime=neu at_pool=default buy"
            ])]
        );
    }

    #[test]
    fn a_leg_normalize_drops_blocks_its_pair() {
        // Unpriced, the swap-in is quarantined in normalize: no event, and
        // its reason.
        let (rows, pairs) = book(
            ("2024-01-01", 100_000_000_000),
            "2024-06-01",
            json!({"rate": null}),
        );
        let response = run(&request(rows, vec![], pairs));
        let detail = swap_detail("missing_spot_price", Some("5E-1"));
        assert_eq!(
            quarantines(&response)[..2],
            [
                json!(["swap-out", "at_swap_basis_carry_unresolved", detail]),
                json!(["swap-in", "at_swap_basis_carry_unresolved", detail]),
            ]
        );
        // An out leg that is also a move's out leg is no event either, and
        // no quarantine names it: the pair's reason is
        // `swap_leg_unavailable`. The second pass books the move at its in
        // leg.
        let move_out = row(
            "move-out",
            "A",
            "outbound",
            "2024-02-01",
            10_000_000_000,
            json!({"anchor": "move", "kind": "transfer"}),
        );
        let move_in = row(
            "move-in",
            "B",
            "inbound",
            "2024-02-01",
            10_000_000_000,
            json!({"anchor": "move", "kind": "transfer"}),
        );
        let usdt_in = row(
            "usdt-in",
            "B",
            "inbound",
            "2024-02-01",
            6_000 * USDT,
            json!({"asset": "USDT", "rate": "1E0"}),
        );
        let pair = cross(&move_out, &usdt_in, Some("swap-1"), json!({}));
        let rows = vec![
            row(
                "buy",
                "A",
                "inbound",
                "2024-01-01",
                100_000_000_000,
                json!({}),
            ),
            move_out,
            move_in,
            usdt_in,
        ];
        let moves = vec![json!({
            "out": "custody-tax:move-out",
            "in": "custody-tax:move-in",
            "source": "reviewed_custody_component",
            "group_id": null,
        })];
        let response = run(&request(rows, moves, vec![pair]));
        let detail = swap_detail("swap_leg_unavailable", Some("1E-1"));
        assert_eq!(
            quarantines(&response),
            vec![
                json!(["move-out", "at_swap_basis_carry_unresolved", detail]),
                json!(["usdt-in", "at_swap_basis_carry_unresolved", detail]),
            ]
        );
        assert_eq!(
            entries(&response),
            vec![
                json!(["acquisition", "buy", "at_regime=neu at_pool=default buy"]),
                json!(["transfer_out"]),
                json!(["transfer_in"]),
            ]
        );
    }

    #[test]
    fn a_blocked_direct_payout_withholds_its_target_disposal() {
        let source = row(
            "payout-source",
            "L",
            "outbound",
            "2025-03-01",
            50_000_000_000,
            json!({"asset": "LBTC", "rate": "50000E0", "kind": "withdrawal"}),
        );
        let payout = |side: &str, direction: &str, kind: &str| {
            row(
                &format!("payout-{side}"),
                "L",
                direction,
                "2025-03-02",
                49_990_000_000,
                json!({
                    "id": format!("direct-payout:payout-1:{side}"),
                    "anchor": "payout-source",
                    "rate": "50000E0",
                    "kind": kind,
                }),
            )
        };
        let (payout_in, payout_out) = (
            payout("in", "inbound", "direct_swap_payout_in"),
            payout("out", "outbound", "sell"),
        );
        let pair = cross(
            &source,
            &payout_in,
            Some("direct-payout:payout-1"),
            json!({"in_transaction_id": null}),
        );
        let rows = vec![
            row(
                "lbtc-buy",
                "L",
                "inbound",
                "2024-06-01",
                40_000_000_000,
                json!({"asset": "LBTC", "rate": "30000E0"}),
            ),
            source,
            payout_in,
            payout_out,
            row(
                "btc-buy",
                "L",
                "inbound",
                "2025-03-03",
                10_000_000_000,
                json!({"rate": "51000E0"}),
            ),
        ];
        let request = request(rows, vec![], vec![pair]);
        let response = run(&request);
        // Both settlement legs anchor to the source, so only the source is
        // quarantined, without its amount; neither leg books.
        assert_eq!(
            quarantines(&response),
            vec![json!([
                "payout-source",
                "at_swap_basis_carry_unresolved",
                {
                    "outgoing_asset": "LBTC",
                    "incoming_asset": "BTC",
                    "at_swap_link": "direct-payout:payout-1",
                    "reason_code": "insufficient_lots",
                }
            ])]
        );
        let booked: Vec<Value> = entries(&response)
            .into_iter()
            .map(|entry| entry[1].clone())
            .collect();
        assert_eq!(booked, [json!("lbtc-buy"), json!("btc-buy")]);
        assert_eq!(
            prepared(&request)["swap_links"]["blocked"],
            json!([
                "custody-tax:payout-source",
                "direct-payout:payout-1:in",
                "direct-payout:payout-1:out",
            ])
        );
    }

    #[test]
    fn a_fallback_id_like_a_direct_payout_withholds_only_the_pair() {
        // A pair without an id whose out row is a direct payout's: the
        // fallback link starts like a direct payout's id, so only the out
        // leg is quarantined, without its amount, but the target-asset
        // disposal is looked up by the pair's own id, which it has none of.
        let swap_out = row(
            "swap-out",
            "A",
            "outbound",
            "2024-06-01",
            200_000_000_000,
            json!({"id": "direct-payout:edge:source", "rate": "60000E0"}),
        );
        let swap_in = row(
            "swap-in",
            "A",
            "inbound",
            "2024-06-01",
            6_000 * USDT,
            json!({"asset": "USDT", "rate": "1E0"}),
        );
        let pair = cross(&swap_out, &swap_in, None, json!({}));
        let rows = vec![
            row(
                "buy",
                "A",
                "inbound",
                "2024-01-01",
                100_000_000_000,
                json!({}),
            ),
            swap_out,
            swap_in,
        ];
        let request = request(rows, vec![], vec![pair]);
        let response = run(&request);
        assert_eq!(
            quarantines(&response),
            vec![json!([
                "direct-payout:edge:source",
                "at_swap_basis_carry_unresolved",
                {
                    "outgoing_asset": "BTC",
                    "incoming_asset": "USDT",
                    "at_swap_link": "direct-payout:edge:source->custody-tax:swap-in",
                    "reason_code": "insufficient_lots",
                }
            ])]
        );
        assert_eq!(
            prepared(&request)["swap_links"]["blocked"],
            json!(["custody-tax:swap-in", "direct-payout:edge:source"])
        );
    }

    #[test]
    fn taxable_pairs_and_pairs_with_a_leg_outside_are_skipped() {
        let rows = swap_rows(("2024-01-01", 100_000_000_000), "2024-06-01", json!({}));
        let taxable = cross(
            &rows[1],
            &rows[2],
            Some("taxable"),
            json!({"policy": "taxable"}),
        );
        let outside = cross(
            &rows[1],
            &rows[2],
            Some("outside"),
            json!({"in_id": "custody-tax:elsewhere"}),
        );
        let request = request(rows, vec![], vec![taxable, outside]);
        let response = run(&request);
        assert_eq!(quarantines(&response), Vec::<Value>::new());
        assert_eq!(response["entries"][1]["at_category"], "neu_gain");
        assert_eq!(
            response["entries"][1]["description"],
            "at_regime=neu at_pool=default swap-out"
        );
        assert_eq!(
            prepared(&request)["swap_links"],
            json!({"links": [], "blocked": [], "quarantines": []})
        );
    }

    #[test]
    fn a_later_pair_relinks_a_shared_row_in_its_place() {
        let mut rows = swap_rows(("2024-01-01", 100_000_000_000), "2024-06-01", json!({}));
        let second = row(
            "second-in",
            "A",
            "inbound",
            "2024-06-01",
            1_000 * USDT,
            json!({"asset": "USDT", "rate": "1E0"}),
        );
        let pairs = vec![
            cross(&rows[1], &rows[2], Some("first"), json!({})),
            cross(&rows[1], &second, Some("second"), json!({})),
        ];
        rows.insert(3, second);
        let request = request(rows, vec![], pairs);
        assert_eq!(
            prepared(&request)["swap_links"]["links"],
            json!([
                ["custody-tax:swap-out", "second"],
                ["custody-tax:swap-in", "first"],
                ["custody-tax:second-in", "second"],
            ])
        );
        // The first pair's in leg is left without its out leg.
        let error = &run(&request)["error"];
        assert_eq!(error["stage"], "validate");
        assert!(error["inner"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Unpaired `at_swap_link=first` marker"));
    }

    #[test]
    fn a_reason_is_found_through_the_pairs_transaction() {
        // Only the pair names a transaction the first pass quarantined: an
        // unpriced purchase after the swap.
        let mut rows = swap_rows(("2024-01-01", 100_000_000_000), "2024-06-01", json!({}));
        rows.push(row(
            "other",
            "A",
            "inbound",
            "2024-07-01",
            1_000_000_000,
            json!({"rate": null}),
        ));
        let pair = cross(
            &rows[1],
            &rows[2],
            Some("swap-1"),
            json!({"out_transaction_id": "other"}),
        );
        let response = run(&request(rows, vec![], vec![pair]));
        let detail = swap_detail("missing_spot_price", Some("5E-1"));
        assert_eq!(
            quarantines(&response)[..3],
            [
                json!(["swap-out", "at_swap_basis_carry_unresolved", detail]),
                json!(["swap-in", "at_swap_basis_carry_unresolved", detail]),
                json!([
                    "other",
                    "missing_spot_price",
                    {"asset": "BTC", "direction": "inbound", "required_for": "acquisition", "wallet": "Cold"}
                ]),
            ]
        );
    }

    #[test]
    fn a_legs_transaction_is_looked_up_before_the_pairs() {
        // The swap-out's transaction has an unpriced slice (normalize's
        // reason, before the gate that then refuses the swap-out after the
        // slice tainted the lot state), and the pair names another
        // transaction, an unclassified income: the slice's reason wins.
        let mut rows = swap_rows(("2024-01-01", 100_000_000_000), "2024-06-01", json!({}));
        rows.push(row(
            "swap-out-2",
            "A",
            "outbound",
            "2024-06-01",
            1_000_000_000,
            json!({"anchor": "swap-out", "rate": null}),
        ));
        rows.push(row(
            "other",
            "A",
            "inbound",
            "2024-07-01",
            1_000_000_000,
            json!({"kind": "cashback_bonus_thing"}),
        ));
        let pair = cross(
            &rows[1],
            &rows[2],
            Some("swap-1"),
            json!({"out_transaction_id": "other"}),
        );
        let response = run(&request(rows, vec![], vec![pair]));
        let detail = swap_detail("missing_spot_price", Some("5E-1"));
        assert_eq!(
            quarantines(&response)[..2],
            [
                json!(["swap-out", "at_swap_basis_carry_unresolved", detail]),
                json!(["swap-in", "at_swap_basis_carry_unresolved", detail]),
            ]
        );
    }

    #[test]
    fn a_link_keeps_its_first_pass_regime_and_the_second_pass_aborts() {
        // On the first pass a swap-in makes the wallet hold Neu, so the
        // sale is Neu and linked. Its pair is blocked (no USDT to swap
        // from), so the second pass sees only Alt: the linked sale is Alt
        // there and no longer a swap leg, which leaves its in leg unpaired.
        let usdt_short = row(
            "usdt-short",
            "A",
            "outbound",
            "2024-01-01",
            30_000 * USDT,
            json!({"asset": "USDT", "rate": "1E0"}),
        );
        let neu_in = row(
            "neu-in",
            "A",
            "inbound",
            "2024-01-01",
            50_000_000_000,
            json!({}),
        );
        let swap_out = row(
            "swap-out",
            "A",
            "outbound",
            "2024-06-01",
            30_000_000_000,
            json!({"rate": "60000E0"}),
        );
        let swap_in = row(
            "swap-in",
            "A",
            "inbound",
            "2024-06-01",
            18_000 * USDT,
            json!({"asset": "USDT", "rate": "1E0"}),
        );
        let pairs = vec![
            cross(
                &usdt_short,
                &neu_in,
                Some("blocked"),
                json!({"out_asset": "USDT", "in_asset": "BTC"}),
            ),
            cross(&swap_out, &swap_in, Some("kept"), json!({})),
        ];
        let rows = vec![
            row(
                "alt",
                "A",
                "inbound",
                "2020-06-01",
                100_000_000_000,
                json!({}),
            ),
            usdt_short,
            neu_in,
            swap_out,
            swap_in,
        ];
        let request = request(rows, vec![], pairs);
        let stages = prepared(&request);
        let event = stages["assets"][0]["normalized"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["row"] == "custody-tax:swap-out")
            .expect("the sale")
            .clone();
        assert_eq!(
            (&event["at_regime"], &event["at_swap_link"]),
            (&json!("alt"), &json!("kept"))
        );
        let error = &run(&request)["error"];
        assert_eq!(error["stage"], "validate");
        assert!(error["inner"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Unpaired `at_swap_link=kept` marker"));
    }
}
