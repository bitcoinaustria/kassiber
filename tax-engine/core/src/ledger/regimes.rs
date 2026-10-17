//! The Austrian regime tracker (design §4.5): `infer_outbound_regimes`
//! (`tests/tax_oracle/austrian.py:141-526`), with the cutoff of
//! `infer_regime_from_timestamp` (`:93-96`), ported literally for the live
//! paths.
//!
//! It runs in each normalize pass of an Austrian book, before normalize
//! visits the rows, over the asset's active rows without a hold, sorted by
//! `occurred_at` as text, inbound before outbound, then id
//! (`tests/tax_oracle/normalize.py:1800-1853`); this order is not the walk's
//! (B map risk 4). It keeps, per wallet, how many msat of Altvermögen and of
//! Neuvermögen are available (a wallet without an id counts as `default`):
//!
//! - an inbound row adds its amount to its time's regime (its fee and its
//!   regime override move nothing);
//! - an outbound row disposes of its time's regime, except that a Neu-time
//!   disposal from a wallet holding Alt but no Neu is Alt, and one from a
//!   wallet holding both records an election (Wahlrecht). It takes its
//!   amount and fee from that regime alone, down to zero;
//! - a move (one without a held leg) is applied at its out leg: it takes
//!   what it sends from its source, from its preferred regime first and then
//!   the other (its time's, or Alt when the source holds Alt but no Neu),
//!   takes its fee from the first slices, and adds the rest, up to what it
//!   receives, to its destination in the same regimes. Its out leg's regime
//!   is its fee's (else the first regime it moved, else the preferred one),
//!   and it records the msat each regime sent and received. Its in leg moves
//!   nothing more. A move whose out leg the pass does not visit is applied
//!   at its in leg.
//!
//! The tracker moves availability even for moves normalize then
//! quarantines (B map §4 quirk 1). Each move is its own component: moves
//! share no leg, so the single-source branch is the only live one
//! (`tests/tax_oracle/austrian.py:290-363`,
//! `tests/tax_oracle/pair_allocation.py:366-420`), and with equal leg
//! amounts no receipt is clamped and none exceeds what was sent.
//!
//! A row's time is read as Kassiber's Austrian rules read it
//! ([`CutoffInstant`]); one Python cannot read fails with Python's own
//! `ValueError`, at the first row the tracker reads it for.

use std::collections::{HashMap, HashSet};

use crate::austria::{DEFAULT_POOL, NEU_CUTOFF_US};
use crate::error::{EngineError, EngineResult, ErrorClass};

use super::request::{CutoffInstant, Direction, Regime, Row};
use super::response::{Flows, RegimeFlows};
use super::run::MovePair;

/// What the tracker found for one pass over an asset.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tracked {
    /// `outbound_regimes`: the regime of each outbound row and of each move's
    /// out leg, by row.
    pub regimes: HashMap<usize, Regime>,
    /// `transfer_regime_flows`: the msat each regime moved, by move (its index
    /// in the request's `intra_pairs`).
    pub flows: HashMap<usize, RegimeFlows>,
    /// `regime_election_row_ids`: the disposals whose Neu regime is an
    /// election, by row.
    pub elected: HashSet<usize>,
}

/// `infer_regime_from_timestamp`: Altvermögen strictly before the cutoff.
pub(crate) fn timestamp_regime(row: &Row) -> EngineResult<Regime> {
    match &row.cutoff_instant {
        CutoffInstant::Read(instant) if instant.us < NEU_CUTOFF_US => Ok(Regime::Alt),
        CutoffInstant::Read(_) => Ok(Regime::Neu),
        CutoffInstant::Unread(unread) => Err(EngineError::new(
            ErrorClass::BuiltinValueError,
            unread.error.clone(),
        )),
    }
}

/// The key of `occurred_at` the sort reads: its text when Python's value is
/// truthy, else `""`.
fn sort_time(row: &Row, set: bool) -> &str {
    if set {
        row.ts.raw()
    } else {
        ""
    }
}

/// `regime_rows`: the `active` rows without a hold, sorted by `occurred_at`
/// as text, inbound before outbound, then id. `is_set` says whether a row's
/// `occurred_at` is truthy in Python.
pub(crate) fn regime_rows(
    rows: &[Row],
    active: &[usize],
    is_set: impl Fn(&Row) -> bool,
) -> Vec<usize> {
    let mut regime_rows: Vec<usize> = active
        .iter()
        .copied()
        .filter(|&index| rows[index].hold.is_none())
        .collect();
    regime_rows.sort_by(|&a, &b| {
        let key = |index: usize| {
            let row = &rows[index];
            (
                sort_time(row, is_set(row)),
                row.direction != Direction::Inbound,
                row.id.as_str(),
            )
        };
        key(a).cmp(&key(b))
    });
    regime_rows
}

/// `_availability_key`: a wallet's id, or `default` for an empty one.
fn wallet_key(row: &Row) -> &str {
    if row.wallet_id.is_empty() {
        DEFAULT_POOL
    } else {
        &row.wallet_id
    }
}

/// The msat of one regime available per wallet.
#[derive(Debug, Default)]
struct Available<'a>(HashMap<&'a str, u128>);

impl<'a> Available<'a> {
    fn get(&self, key: &str) -> u128 {
        self.0.get(key).copied().unwrap_or(0)
    }

    fn add(&mut self, key: &'a str, amount: u128) {
        *self.0.entry(key).or_insert(0) += amount;
    }

    /// `_deplete_available`: takes up to `amount` and returns what it took.
    fn take(&mut self, key: &'a str, amount: u128) -> u128 {
        if amount == 0 {
            return 0;
        }
        let available = self.get(key);
        let taken = available.min(amount);
        self.0.insert(key, available - taken);
        taken
    }
}

/// Per-regime msat, indexed `[alt, neu]`.
type PerRegime = [u128; 2];

fn slot(regime: Regime) -> usize {
    match regime {
        Regime::Alt => 0,
        Regime::Neu => 1,
    }
}

struct Tracker<'a> {
    rows: &'a [Row],
    alt: Available<'a>,
    neu: Available<'a>,
    out: Tracked,
}

impl<'a> Tracker<'a> {
    fn bucket(&mut self, regime: Regime) -> &mut Available<'a> {
        match regime {
            Regime::Alt => &mut self.alt,
            Regime::Neu => &mut self.neu,
        }
    }

    /// `_move_transfer_availability`: the move's fee regime and flows.
    fn apply_move(&mut self, pair: &MovePair) -> EngineResult<(Regime, RegimeFlows)> {
        let rows = self.rows;
        let (out_row, in_row) = (&rows[pair.out], &rows[pair.inbound]);
        let (from, to) = (wallet_key(out_row), wallet_key(in_row));
        // The action's out leg sends what it receives plus the fee
        // allocated to it, which is all of the out leg's amount and fee.
        let sent = u128::from(out_row.amount_msat) + u128::from(out_row.fee_msat);
        let received = u128::from(in_row.amount_msat);
        let mut preferred = timestamp_regime(out_row)?;
        if sent == 0 || received == 0 {
            return Ok((preferred, flows([0, 0], [0, 0])));
        }
        if preferred == Regime::Neu && self.alt.get(from) > 0 && self.neu.get(from) == 0 {
            preferred = Regime::Alt;
        }
        let order = match preferred {
            Regime::Neu => [Regime::Neu, Regime::Alt],
            Regime::Alt => [Regime::Alt, Regime::Neu],
        };
        let mut moved: PerRegime = [0, 0];
        let mut remaining = sent;
        for regime in order {
            let taken = self.bucket(regime).take(from, remaining);
            moved[slot(regime)] += taken;
            remaining -= taken;
            if remaining == 0 {
                break;
            }
        }
        // The fee is the first slice of what left the source.
        let mut fee_by: PerRegime = [0, 0];
        let mut remaining_fee = sent.saturating_sub(received);
        for regime in order {
            let fee_slice = moved[slot(regime)].min(remaining_fee);
            if fee_slice == 0 {
                continue;
            }
            moved[slot(regime)] -= fee_slice;
            fee_by[slot(regime)] += fee_slice;
            remaining_fee -= fee_slice;
            if remaining_fee == 0 {
                break;
            }
        }
        // The rest reaches the destination, up to what it received.
        let mut carried: PerRegime = [0, 0];
        let mut remaining_received = received;
        for regime in order {
            let carry = moved[slot(regime)].min(remaining_received);
            if carry == 0 {
                continue;
            }
            self.bucket(regime).add(to, carry);
            carried[slot(regime)] += carry;
            remaining_received -= carry;
            if remaining_received == 0 {
                break;
            }
        }
        let sent_by = [fee_by[0] + carried[0], fee_by[1] + carried[1]];
        let flows = flows(sent_by, carried);
        let fee_regime = order
            .iter()
            .find(|&&regime| fee_by[slot(regime)] > 0)
            .or_else(|| order.iter().find(|&&regime| moved[slot(regime)] > 0))
            .copied()
            .unwrap_or(preferred);
        Ok((fee_regime, flows))
    }

    /// A standalone row (`infer_outbound_regimes`, `:496-524`).
    fn standalone(&mut self, index: usize) -> EngineResult<()> {
        let row = &self.rows[index];
        let key = wallet_key(row);
        let regime = timestamp_regime(row)?;
        match row.direction {
            Direction::Inbound => {
                self.bucket(regime).add(key, row.amount_msat.into());
            }
            Direction::Outbound => {
                let mut regime = regime;
                if regime == Regime::Neu && self.alt.get(key) > 0 {
                    if self.neu.get(key) == 0 {
                        regime = Regime::Alt;
                    } else {
                        // Both regimes are available: Neu first is an
                        // exercised designation, not a forced outcome.
                        self.out.elected.insert(index);
                    }
                }
                self.out.regimes.insert(index, regime);
                let needed = u128::from(row.amount_msat) + u128::from(row.fee_msat);
                // A disposal depletes only its own regime; a shortfall is
                // dropped.
                self.bucket(regime).take(key, needed);
            }
        }
        Ok(())
    }
}

fn flows(sent: PerRegime, received: PerRegime) -> RegimeFlows {
    RegimeFlows {
        out: Flows {
            alt: sent[0],
            neu: sent[1],
        },
        inbound: Flows {
            alt: received[0],
            neu: received[1],
        },
    }
}

/// `infer_outbound_regimes` over `regime_rows` (sorted, see
/// [`regime_rows`]) and the asset's `moves` without a held leg, in request
/// order.
pub(crate) fn track(
    rows: &[Row],
    regime_rows: &[usize],
    moves: &[&MovePair],
) -> EngineResult<Tracked> {
    let visited: HashSet<usize> = regime_rows.iter().copied().collect();
    // `_transfer_actions_for_intra_pairs`: each move is applied at its out
    // leg when the pass visits it, else at its in leg. Moves share no leg,
    // so a row triggers at most one.
    let mut actions: HashMap<usize, &MovePair> = HashMap::new();
    let mut legs: HashSet<usize> = HashSet::new();
    for &pair in moves {
        legs.insert(pair.out);
        legs.insert(pair.inbound);
        let trigger = if visited.contains(&pair.out) {
            pair.out
        } else {
            pair.inbound
        };
        actions.insert(trigger, pair);
    }
    let mut tracker = Tracker {
        rows,
        alt: Available::default(),
        neu: Available::default(),
        out: Tracked::default(),
    };
    for &index in regime_rows {
        if let Some(&pair) = actions.get(&index) {
            let (regime, flows) = tracker.apply_move(pair)?;
            // The move's taxable fee needs a regime; the out leg keeps the
            // first one it is given.
            tracker.out.regimes.entry(pair.out).or_insert(regime);
            tracker.out.flows.insert(pair.index, flows);
            continue;
        }
        if legs.contains(&index) {
            continue;
        }
        tracker.standalone(index)?;
    }
    Ok(tracker.out)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;
    use crate::ledger::request::LedgerRequest;
    use crate::ledger::run::Book;

    // The single-pair cases of `tests/test_austrian_classification.py`'s
    // former `InferOutboundRegimesTest` (`:76-118`, `:120`, `:296`), on the
    // tracker itself. Each expectation is what the oracle's
    // `infer_outbound_regimes` returns for the same rows (scratch replay,
    // recorded in the implementation log).

    /// 2021-02-28T22:59:59.999999Z, the last Alt instant, in microseconds.
    const LAST_ALT_US: i64 = NEU_CUTOFF_US - 1;

    fn row(id: &str, wallet: &str, direction: &str, raw: &str, us: i64, amount: u64) -> Value {
        json!({
            "id": format!("custody-tax:{id}"),
            "anchor": id,
            "asset": "BTC",
            "wallet_id": wallet,
            "direction": direction,
            "ts": {"raw": raw, "error": "not read here"},
            "cutoff_instant": {"us": us},
            "amount_msat": amount,
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
            "chain_order": null,
        })
    }

    fn request(rows: Vec<Value>, pairs: Vec<(&str, &str)>) -> LedgerRequest {
        serde_json::from_value(json!({
            "schema_version": 2,
            "operation": "prepare",
            "policy": {
                "country": {"kind": "at"},
                "method": "moving_average_at",
                "method_label": "moving_average_at",
                "require_coarse_review": false,
            },
            "config": {"assets": ["BTC"], "exchanges": ["Cold", "Exchange", "Hot"], "holders": ["Default"]},
            "holder": "Default",
            "wallets": [
                {"id": "A", "label": "Cold"},
                {"id": "B", "label": "Hot"},
                {"id": "C", "label": "Exchange"},
                {"id": "", "label": "Unnamed"},
            ],
            "rows": rows,
            "intra_pairs": pairs.into_iter().map(|(out, inbound)| json!({
                "out": format!("custody-tax:{out}"),
                "in": format!("custody-tax:{inbound}"),
                "source": null,
                "group_id": null,
            })).collect::<Vec<_>>(),
            "cross_asset_pairs": [],
        }))
        .expect("a request")
    }

    /// The tracker over every row of the book, as one pass sees it.
    fn tracked(request: &LedgerRequest) -> EngineResult<Tracked> {
        let book = Book::new(request).expect("a book");
        let part = &book.assets[0];
        let order = regime_rows(book.rows, &part.rows, |row| !row.ts.raw().is_empty());
        let moves: Vec<&MovePair> = part
            .pairs
            .iter()
            .filter(|pair| {
                book.rows[pair.out].hold.is_none() && book.rows[pair.inbound].hold.is_none()
            })
            .collect();
        track(book.rows, &order, &moves)
    }

    fn index(request: &LedgerRequest, id: &str) -> usize {
        let id = format!("custody-tax:{id}");
        request
            .rows
            .iter()
            .position(|row| row.id == id)
            .expect("a row")
    }

    fn regime_of(request: &LedgerRequest, tracked: &Tracked, id: &str) -> Option<Regime> {
        tracked.regimes.get(&index(request, id)).copied()
    }

    const ALT: &str = "2020-06-01T00:00:00Z";
    const ALT_US: i64 = 1_590_969_600_000_000;
    const NEU: &str = "2022-06-01T00:00:00Z";
    const NEU_US: i64 = 1_654_041_600_000_000;
    const LATER: &str = "2023-06-01T00:00:00Z";
    const LATER_US: i64 = 1_685_577_600_000_000;

    #[test]
    fn the_cutoff_is_strict_and_an_unread_time_fails_with_pythons_value_error() {
        let mut rows = vec![row("a", "A", "inbound", ALT, LAST_ALT_US, 1)];
        rows.push(row("b", "A", "inbound", NEU, NEU_CUTOFF_US, 1));
        let request = request(rows, Vec::new());
        assert_eq!(timestamp_regime(&request.rows[0]).unwrap(), Regime::Alt);
        assert_eq!(timestamp_regime(&request.rows[1]).unwrap(), Regime::Neu);
        let mut unread = row("c", "A", "inbound", "x", 0, 1);
        unread["cutoff_instant"] = json!({"error": "Invalid isoformat string: 'x'"});
        let request = self::request(vec![unread], Vec::new());
        let error = timestamp_regime(&request.rows[0]).unwrap_err();
        assert_eq!(error.class, ErrorClass::BuiltinValueError);
        assert_eq!(error.message, "Invalid isoformat string: 'x'");
        // The tracker fails at that row.
        assert_eq!(tracked(&request).unwrap_err(), error);
    }

    #[test]
    fn a_post_cutoff_sale_from_an_alt_only_wallet_is_alt() {
        // `:76`: only Alt inventory in the disposing wallet.
        let request = request(
            vec![
                row("buy", "A", "inbound", ALT, ALT_US, 100),
                row("sell", "A", "outbound", NEU, NEU_US, 40),
            ],
            Vec::new(),
        );
        let tracked = tracked(&request).unwrap();
        assert_eq!(regime_of(&request, &tracked, "sell"), Some(Regime::Alt));
        assert!(tracked.elected.is_empty());
    }

    #[test]
    fn a_sale_from_a_mixed_wallet_elects_neu() {
        // `:83`: both regimes available, so Neu is a designation.
        let request = request(
            vec![
                row("alt", "A", "inbound", ALT, ALT_US, 100),
                row("neu", "A", "inbound", NEU, NEU_US, 100),
                row("sell", "A", "outbound", LATER, LATER_US, 40),
            ],
            Vec::new(),
        );
        let tracked = tracked(&request).unwrap();
        assert_eq!(regime_of(&request, &tracked, "sell"), Some(Regime::Neu));
        assert_eq!(
            tracked.elected,
            [index(&request, "sell")].into_iter().collect()
        );
    }

    #[test]
    fn a_sale_from_a_neu_only_wallet_is_forced_neu() {
        // Only Neu in the wallet: Neu without an election, and an
        // unfunded Alt-time sale keeps its time's regime.
        let request = request(
            vec![
                row("neu", "A", "inbound", NEU, NEU_US, 100),
                row("sell", "A", "outbound", LATER, LATER_US, 40),
                row("early", "B", "outbound", ALT, ALT_US, 5),
            ],
            Vec::new(),
        );
        let tracked = tracked(&request).unwrap();
        assert_eq!(regime_of(&request, &tracked, "sell"), Some(Regime::Neu));
        assert_eq!(regime_of(&request, &tracked, "early"), Some(Regime::Alt));
        assert!(tracked.elected.is_empty());
    }

    #[test]
    fn a_move_whose_fee_takes_the_last_neu_is_neu() {
        // The fee's first slice is the source's last Neu, so the move's
        // regime is Neu although only Alt reaches the destination.
        let mut out = row("m-out", "A", "outbound", LATER, LATER_US, 50);
        out["fee_msat"] = json!(10);
        let request = request(
            vec![
                row("alt", "A", "inbound", ALT, ALT_US, 100),
                row("neu", "A", "inbound", NEU, NEU_US, 10),
                out,
                row("m-in", "B", "inbound", LATER, LATER_US, 50),
            ],
            vec![("m-out", "m-in")],
        );
        let tracked = tracked(&request).unwrap();
        assert_eq!(tracked.flows[&0], flows([50, 10], [50, 0]));
        assert_eq!(regime_of(&request, &tracked, "m-out"), Some(Regime::Neu));
    }

    #[test]
    fn a_move_applies_at_its_out_legs_time() {
        // The move leaves A before the sale and arrives after it: applied at
        // its out leg, it takes A's Neu first, so the sale finds only Alt.
        let request = request(
            vec![
                row("alt", "A", "inbound", ALT, ALT_US, 100),
                row("neu", "A", "inbound", NEU, NEU_US, 50),
                row("m-out", "A", "outbound", LATER, LATER_US, 50),
                row(
                    "sell",
                    "A",
                    "outbound",
                    "2023-06-01T00:05:00Z",
                    LATER_US + 300_000_000,
                    10,
                ),
                row(
                    "m-in",
                    "B",
                    "inbound",
                    "2023-06-01T00:10:00Z",
                    LATER_US + 600_000_000,
                    50,
                ),
            ],
            vec![("m-out", "m-in")],
        );
        let tracked = tracked(&request).unwrap();
        assert_eq!(tracked.flows[&0], flows([0, 50], [0, 50]));
        assert_eq!(regime_of(&request, &tracked, "sell"), Some(Regime::Alt));
    }

    #[test]
    fn availability_is_per_wallet_and_a_disposal_depletes_only_its_regime() {
        // `:91`: Neu in another wallet does not make this one's sale Neu.
        // `:99`: a Neu sale larger than the Neu left drains Neu only, so the
        // next sale is Alt.
        let request = request(
            vec![
                row("alt", "A", "inbound", ALT, ALT_US, 100),
                row("neu-b", "B", "inbound", NEU, NEU_US, 100),
                row(
                    "sell-a",
                    "A",
                    "outbound",
                    "2022-06-01T00:00:01Z",
                    NEU_US + 1_000_000,
                    10,
                ),
                row("neu-a", "A", "inbound", LATER, LATER_US, 10),
                row(
                    "drain",
                    "A",
                    "outbound",
                    "2023-06-01T00:00:01Z",
                    LATER_US + 1_000_000,
                    50,
                ),
                row(
                    "after",
                    "A",
                    "outbound",
                    "2023-06-01T00:00:02Z",
                    LATER_US + 2_000_000,
                    5,
                ),
            ],
            Vec::new(),
        );
        let tracked = tracked(&request).unwrap();
        assert_eq!(regime_of(&request, &tracked, "sell-a"), Some(Regime::Alt));
        assert_eq!(regime_of(&request, &tracked, "drain"), Some(Regime::Neu));
        assert_eq!(regime_of(&request, &tracked, "after"), Some(Regime::Alt));
        assert!(tracked.elected.contains(&index(&request, "drain")));
        assert!(!tracked.elected.contains(&index(&request, "after")));
    }

    #[test]
    fn a_move_carries_regimes_spills_and_pays_its_fee_first() {
        // `:120`: a move spends Neu first, then Alt, takes its fee from the
        // first slice, and its destination then sells what it carried.
        let mut out = row("move-out", "A", "outbound", LATER, LATER_US, 150);
        out["fee_msat"] = json!(20);
        let request = request(
            vec![
                row("alt", "A", "inbound", ALT, ALT_US, 100),
                row("neu", "A", "inbound", NEU, NEU_US, 100),
                out,
                row("move-in", "B", "inbound", LATER, LATER_US, 150),
                row("sell-b", "B", "outbound", LATER, LATER_US + 1, 10),
            ],
            vec![("move-out", "move-in")],
        );
        let tracked = tracked(&request).unwrap();
        assert_eq!(regime_of(&request, &tracked, "move-out"), Some(Regime::Neu));
        let pair = 0;
        assert_eq!(
            tracked.flows[&pair],
            flows([70, 100], [70, 80]),
            "Neu 100 (20 fee, 80 carried), then 70 Alt carried"
        );
        // B now holds both regimes: its sale elects Neu.
        assert_eq!(regime_of(&request, &tracked, "sell-b"), Some(Regime::Neu));
        assert!(tracked.elected.contains(&index(&request, "sell-b")));
        // The in leg moves nothing more and gets no regime.
        assert_eq!(regime_of(&request, &tracked, "move-in"), None);
    }

    #[test]
    fn an_alt_only_move_is_alt_and_a_move_from_nothing_keeps_its_preference() {
        // `:296`: a post-cutoff move out of an Alt-only wallet carries Alt.
        let request = request(
            vec![
                row("alt", "A", "inbound", ALT, ALT_US, 100),
                row("m-out", "A", "outbound", NEU, NEU_US, 50),
                row("m-in", "B", "inbound", NEU, NEU_US, 50),
                row("e-out", "C", "outbound", NEU, NEU_US, 5),
                row("e-in", "B", "inbound", NEU, NEU_US, 5),
            ],
            vec![("m-out", "m-in"), ("e-out", "e-in")],
        );
        let tracked = tracked(&request).unwrap();
        assert_eq!(regime_of(&request, &tracked, "m-out"), Some(Regime::Alt));
        assert_eq!(tracked.flows[&0], flows([50, 0], [50, 0]));
        // An empty source moves nothing: the regime is the move's time's.
        assert_eq!(regime_of(&request, &tracked, "e-out"), Some(Regime::Neu));
        assert_eq!(tracked.flows[&1], flows([0, 0], [0, 0]));
    }

    #[test]
    fn rows_sort_by_text_time_then_inbound_first_then_id() {
        // A same-time sale and purchase: the purchase counts first, though
        // the sale's id sorts first. The text decides, so the same instant
        // spelled `+00:00` sorts before `Z`, and a held row is left out.
        let request = request(
            vec![
                row("a-sell", "A", "outbound", NEU, NEU_US, 10),
                row("b-buy", "A", "inbound", NEU, NEU_US, 10),
                row(
                    "z-sell",
                    "A",
                    "outbound",
                    "2022-06-01T00:00:00+00:00",
                    NEU_US,
                    10,
                ),
                row("held", "A", "inbound", ALT, ALT_US, 10),
            ],
            Vec::new(),
        );
        let book = Book::new(&request).unwrap();
        let mut rows = request.rows.clone();
        rows[3].hold = Some(crate::ledger::request::Hold::ConflictingSpend);
        let ids = |order: Vec<usize>| -> Vec<String> {
            order.iter().map(|&index| rows[index].id.clone()).collect()
        };
        assert_eq!(
            ids(regime_rows(&rows, &book.assets[0].rows, |_| true)),
            [
                "custody-tax:z-sell",
                "custody-tax:b-buy",
                "custody-tax:a-sell"
            ]
        );
        // A row whose time is falsy sorts as `""`, first.
        assert_eq!(
            ids(regime_rows(&rows, &book.assets[0].rows, |row| {
                row.id != "custody-tax:a-sell"
            })),
            [
                "custody-tax:a-sell",
                "custody-tax:z-sell",
                "custody-tax:b-buy"
            ]
        );
    }

    #[test]
    fn a_move_whose_out_leg_is_not_visited_applies_at_its_in_leg() {
        let request = request(
            vec![
                row("neu", "A", "inbound", NEU, NEU_US, 100),
                row("m-out", "A", "outbound", NEU, NEU_US, 50),
                row(
                    "sell-a",
                    "A",
                    "outbound",
                    "2022-12-01T00:00:00Z",
                    NEU_US + 1,
                    60,
                ),
                row("m-in", "B", "inbound", LATER, LATER_US, 50),
            ],
            vec![("m-out", "m-in")],
        );
        let book = Book::new(&request).unwrap();
        let part = &book.assets[0];
        let active: Vec<usize> = part
            .rows
            .iter()
            .copied()
            .filter(|&index| request.rows[index].id != "custody-tax:m-out")
            .collect();
        let order = regime_rows(book.rows, &active, |_| true);
        let moves: Vec<&MovePair> = part.pairs.iter().collect();
        let tracked = track(book.rows, &order, &moves).unwrap();
        // Applied at its in leg's later time, the move finds only the Neu
        // the sale left in A, and moves that.
        assert_eq!(tracked.flows[&0], flows([0, 40], [0, 40]));
        assert_eq!(regime_of(&request, &tracked, "m-out"), Some(Regime::Neu));
        // The wallet without an id counts as `default`.
        let key_row = row("x", "", "inbound", NEU, NEU_US, 1);
        let request = self::request(vec![key_row], Vec::new());
        assert_eq!(wallet_key(&request.rows[0]), "default");
    }
}
