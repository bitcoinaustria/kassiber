//! The generic Bitcoin rail carry (design §4.10): the adapter's
//! `_apply_generic_bitcoin_rail_carry_values`
//! (`tests/tax_oracle/adapter.py:2051-2166`), with
//! `_decimal_to_row_value` and `_with_carried_fiat_value` (`:1961-1982`),
//! `_computed_cost_basis_by_transaction_id` (`:1985-1994`), and
//! `_generic_bitcoin_rail_pair_quarantines` (`:2020-2048`), ported
//! literally.
//!
//! On a generic book, a carrying-value cross-asset pair between BTC and
//! LBTC whose legs are both rows moves the out leg's cost basis to its in
//! leg: the in leg is acquired at what the out leg's lots cost, and the out
//! leg is sold for the same, so the rail change realizes nothing. That basis
//! is known only once the lot engine has priced the out leg with the book's
//! method, and a carried acquisition can be the source of the next hop, so
//! the carry is a fixed-point loop. Each pass prepares and computes the
//! whole book, over the rows no blocked pair withholds, as the final run
//! does (normalize, the walk, the method gate, and the lot engine; not the
//! country's checks), then takes each eligible pair in request order:
//!
//! - a pair with a blocked row is left as it is;
//! - a pair whose out leg has no fragment basis, or one that is not
//!   positive, is blocked for good: its rows, and a direct payout's
//!   target-asset disposal, leave every later pass, for the first reason
//!   the pass's quarantines give the out leg's transaction (only the source
//!   explains a missing basis; an unpriced in leg is what the carry would
//!   fix), else `source_basis_unavailable`;
//! - otherwise both legs are priced at the carried basis, as exact pricing:
//!   the value `format(basis.normalize(), "f")` and the rate
//!   `format((basis / amount).normalize(), "f")`, none for a row without an
//!   amount.
//!
//! The loop runs at most one pass more than there are eligible pairs and
//! stops after a pass that changed no row's carried pricing and blocked no
//! new pair; running out of passes is not an error, so a book whose carries
//! do not settle keeps the last pass's pricing (design §4.10, D-carry-2). A
//! row counts as changed when no earlier pass priced it or its strings
//! differ: the adapter compares the whole row, and a projection row stores
//! its value as `None` or a float, never as the carried text. The blocked
//! pairs' quarantines come after the loop, in pair order, on both legs.
//!
//! An Austrian book carries nothing here: its carrying-value pairs are swap
//! links ([`super::swaplinks`]).

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{Map, Value};

use crate::decimal::{Context, Decimal};
use crate::error::{EngineError, EngineResult};
use crate::manifest::{DetailKind, QuarantineReason, RAIL_QUARANTINE_REASON};
use crate::model::ConfigSpec;
use crate::num;

use super::normalize::{btc, Quarantine};
use super::request::{Country, CrossAssetPair, DecimalText, LedgerRequest, Row};
use super::response::{Stage, StagedError};
use super::run::{compute, lot_inputs, prepare_assets, Book, Pass, Stages};
use super::swaplinks::{blocked_rows, link_id, or_else, LegReasons, CARRYING_VALUE};

/// The reason code of a blocked pair whose out leg has no basis and no
/// quarantine naming its transaction.
pub(crate) const SOURCE_BASIS_UNAVAILABLE: &str = "source_basis_unavailable";

/// A carried row's pricing: its value and rate as the adapter stores them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Carried {
    pub value: String,
    /// `None` for a row without an amount.
    pub rate: Option<String>,
}

/// What the carry leaves the final pass.
#[derive(Clone, Debug, Default)]
pub(crate) struct Carry {
    /// Each row a pass priced, by index, at its last carried pricing.
    pub carried: BTreeMap<usize, Carried>,
    /// The ids of the rows the blocked pairs withhold, a direct payout's
    /// `<pair>:out` whether or not a row has it.
    pub blocked: BTreeSet<String>,
    /// The blocked pairs' quarantines, in pair order.
    pub quarantines: Vec<Quarantine>,
    /// The passes it ran.
    pub passes: usize,
}

impl Carry {
    /// The request the final pass reads: the carried rows priced as the
    /// carry left them (the adapter's `rows_for_engine` after the carry).
    /// Reading the carried pricing fails at stage `prepare`, raised bare, as
    /// inside the carry's passes (the final pass reads the last pass's
    /// pricing when the passes ran out).
    pub fn request<'r>(
        &self,
        request: &'r LedgerRequest,
    ) -> Result<Cow<'r, LedgerRequest>, StagedError> {
        carried_request(request, &self.carried)
            .map_err(|error| StagedError::new(Stage::Prepare, error))
    }
}

/// A carry pass that failed, and the stages that pass reached.
pub(crate) struct CarryFailure {
    pub error: StagedError,
    pub stages: Vec<Stages>,
}

impl CarryFailure {
    fn new(error: StagedError) -> Self {
        CarryFailure {
            error,
            stages: Vec::new(),
        }
    }
}

/// The carry over `book`. A pass that fails fails the run, at its stage.
pub(crate) fn carry(book: &Book, config: &ConfigSpec) -> Result<Carry, CarryFailure> {
    let mut carry = Carry::default();
    if matches!(book.request.policy.country, Country::At {}) {
        return Ok(carry);
    }
    // The carrying-value Bitcoin rail pairs whose legs are both rows.
    let eligible: Vec<(&CrossAssetPair, usize, usize)> = book
        .request
        .cross_asset_pairs
        .iter()
        .filter(|pair| pair.policy.as_deref() == Some(CARRYING_VALUE) && pair.bitcoin_rail)
        .filter_map(|pair| {
            Some((
                pair,
                book.row_index(&pair.out_id)?,
                book.row_index(&pair.in_id)?,
            ))
        })
        .collect();
    if eligible.is_empty() {
        return Ok(carry);
    }
    let bare = |error| CarryFailure::new(StagedError::new(Stage::Prepare, error));
    // Why each blocked pair was blocked, by its key (`blocked_pair_reasons`).
    let mut reasons_by_key: HashMap<String, &'static str> = HashMap::new();
    let max_passes = (eligible.len() + 1).max(1);
    for _ in 0..max_passes {
        carry.passes += 1;
        let request = carried_request(book.request, &carry.carried).map_err(bare)?;
        let pass_book =
            Book::new(&request).map_err(|error| CarryFailure::new(StagedError::contract(error)))?;
        let pass = Pass {
            excluded: carry
                .blocked
                .iter()
                .filter_map(|id| pass_book.row_index(id))
                .collect(),
            links: HashMap::new(),
        };
        let (stages, error) = prepare_assets(&pass_book, config, &pass);
        if let Some(error) = error {
            return Err(CarryFailure {
                error: StagedError::new(Stage::Prepare, error),
                stages,
            });
        }
        let basis = match computed_basis(&pass_book, &stages) {
            Ok(basis) => basis,
            Err(error) => return Err(CarryFailure { error, stages }),
        };
        let mut reasons = LegReasons::new(pass_book.rows);
        for stages in &stages {
            if let Some(walked) = &stages.walked {
                reasons.add(&stages.normalized, walked);
            }
        }
        let mut overrides: BTreeMap<usize, Carried> = BTreeMap::new();
        let mut rows_changed = false;
        let mut blocks_changed = false;
        for &(pair, out, inbound) in &eligible {
            if carry.blocked.contains(&pair.out_id) || carry.blocked.contains(&pair.in_id) {
                continue;
            }
            let Some(carried_basis) = basis.get(&pair.out_id).filter(|basis| basis.is_positive())
            else {
                // Only the source leg explains a missing carried basis.
                let reason_code = reasons
                    .leg(out, pair.out_transaction_id.as_deref())
                    .map_or(SOURCE_BASIS_UNAVAILABLE, QuarantineReason::as_str);
                let withheld = blocked_rows(pair);
                if !withheld.iter().all(|id| carry.blocked.contains(id)) {
                    blocks_changed = true;
                    carry.blocked.extend(withheld);
                }
                reasons_by_key.insert(link_id(pair), reason_code);
                continue;
            };
            for row in [out, inbound] {
                let carried = carried_pricing(&pass_book.rows[row], carried_basis).map_err(bare)?;
                if carry.carried.get(&row) != Some(&carried) {
                    rows_changed = true;
                }
                overrides.insert(row, carried);
            }
        }
        if !overrides.is_empty() && rows_changed {
            carry.carried.extend(overrides);
        }
        if !rows_changed && !blocks_changed {
            break;
        }
    }
    for &(pair, out, inbound) in &eligible {
        if let Some(reason_code) = reasons_by_key.get(&link_id(pair)) {
            carry
                .quarantines
                .extend(rail_quarantines(book, pair, out, inbound, reason_code));
        }
    }
    Ok(carry)
}

/// One pass's lot engine over its walked assets, and each taxable event's
/// fragment basis, summed from `Decimal("0")` in the lot engine's order, by
/// the event's unique id (its row's id; a move's out leg's)
/// (`_computed_cost_basis_by_transaction_id`).
fn computed_basis(book: &Book, stages: &[Stages]) -> Result<HashMap<String, Decimal>, StagedError> {
    let inputs = lot_inputs(book, stages);
    let outputs = compute(book, &inputs)?;
    let bare = |error| StagedError::new(Stage::Prepare, error);
    let mut basis: HashMap<String, Decimal> = HashMap::new();
    let computed = stages
        .iter()
        .filter_map(|stages| stages.walked.as_ref()?.entries.as_ref());
    for (entries, output) in computed.zip(&outputs) {
        let events: HashMap<i64, &str> = entries
            .iter()
            .map(|built| (built.entry.row, built.entry.unique_id.as_str()))
            .collect();
        for gain_loss in &output.gain_losses {
            let unique_id = events.get(&gain_loss.event.row).ok_or_else(|| {
                StagedError::new(
                    Stage::Classify,
                    EngineError::runtime(
                        "Internal error: the lot engine returned an unknown entry",
                    ),
                )
            })?;
            let sum = basis
                .entry((*unique_id).to_owned())
                .or_insert_with(num::zero);
            *sum = num::add(sum, &gain_loss.fiat_cost_basis.0).map_err(bare)?;
        }
    }
    Ok(basis)
}

/// `_with_carried_fiat_value`'s value and rate: the basis, and the basis
/// per unit of the row's amount, each `format(x.normalize(), "f")`
/// (`_decimal_to_row_value`) in RP2's context, the thread's.
fn carried_pricing(row: &Row, basis: &Decimal) -> EngineResult<Carried> {
    let value = row_value(basis)?;
    let quantity = btc(row.amount_msat.into())?;
    let rate = if quantity.is_positive() {
        Some(row_value(&num::div(basis, &quantity)?)?)
    } else {
        None
    };
    Ok(Carried { value, rate })
}

fn row_value(value: &Decimal) -> EngineResult<String> {
    Ok(value.normalize(&Context::RP2)?.to_fixed_string()?)
}

/// The request with each carried row priced as the adapter rewrote it:
/// `decimal_from_exact` of its fixed strings (which parse exactly) for the
/// rate and value, and exact pricing, so not coarse.
fn carried_request<'r>(
    request: &'r LedgerRequest,
    carried: &BTreeMap<usize, Carried>,
) -> EngineResult<Cow<'r, LedgerRequest>> {
    if carried.is_empty() {
        return Ok(Cow::Borrowed(request));
    }
    let stored =
        |text: &str| -> EngineResult<DecimalText> { Ok(DecimalText::Finite(text.parse()?)) };
    let mut request = request.clone();
    for (&row, pricing) in carried {
        let row = &mut request.rows[row];
        row.rate = pricing.rate.as_deref().map(stored).transpose()?;
        row.value = Some(stored(&pricing.value)?);
        row.coarse = false;
    }
    Ok(Cow::Owned(request))
}

/// `_generic_bitcoin_rail_pair_quarantines`: both legs, one detail.
fn rail_quarantines(
    book: &Book,
    pair: &CrossAssetPair,
    out: usize,
    inbound: usize,
    reason_code: &str,
) -> Vec<Quarantine> {
    let rows = book.rows;
    let detail: Map<String, Value> = [
        (
            "outgoing_asset",
            Value::from(or_else(pair.out_asset.as_deref(), &rows[out].asset)),
        ),
        (
            "incoming_asset",
            Value::from(or_else(pair.in_asset.as_deref(), &rows[inbound].asset)),
        ),
        ("rail_pair", Value::from(link_id(pair))),
        ("policy", Value::from(CARRYING_VALUE)),
        ("reason_code", Value::from(reason_code)),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect();
    [out, inbound]
        .into_iter()
        .map(|row| Quarantine {
            row,
            reason: RAIL_QUARANTINE_REASON,
            detail_kind: DetailKind::Plain,
            detail: detail.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::ledger::request::LedgerRequest;
    use crate::ledger::run::{ledger, prepare};
    use crate::time::CivilDate;

    // Small generic books through the whole pipeline. Every expectation is
    // what the oracle gives for the same rows (projections built with
    // `tests/tax_ledger_generator.py`'s `projected` and `direct`, compared
    // with `tests/tax_engine_compare.py`: production equals the oracle on
    // each, and the carry's passes, priced rows, and blocks were read from
    // the oracle's own carry); `tests/test_tax_ledger_prepare.py` compares
    // the carry and the final pass with the oracle's on the goldens, seeded
    // books, and edge books.

    /// 1 BTC in msat.
    const BTC: u64 = 100_000_000_000;

    /// A UTC time, `YYYY-MM-DDTHH:MM:SSZ`, as the builder sends it.
    fn ts(stamp: &str) -> Value {
        let date = &stamp[..10];
        let time = &stamp[11..19];
        let field = |at: usize| stamp[at..at + 2].parse::<i64>().expect("a time");
        let days = CivilDate::parse(date).expect("a date").to_days();
        let seconds = days * 86_400 + field(11) * 3_600 + field(14) * 60 + field(17);
        json!({
            "raw": stamp,
            "us": seconds * 1_000_000,
            "offset_s": 0,
            "date": date,
            "year": date[..4].parse::<i32>().expect("a year"),
            "vienna_date": date,
            "display": format!("{date} {time}+00:00"),
            "iso": format!("{date}T{time}+00:00"),
        })
    }

    /// A row named `name` (its id `custody-tax:<name>`, its transaction and
    /// text `name`), a BTC purchase or sale priced at 40000 unless `fields`
    /// says otherwise.
    fn row(
        name: &str,
        wallet: &str,
        direction: &str,
        stamp: &str,
        amount: u64,
        fields: Value,
    ) -> Value {
        let time = ts(stamp);
        let mut row = json!({
            "id": format!("custody-tax:{name}"),
            "anchor": name,
            "asset": "BTC",
            "wallet_id": wallet,
            "direction": direction,
            "cutoff_instant": {"us": time["us"]},
            "ts": time,
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

    /// A cross-asset pair between two rows, as the builder sends one.
    fn cross(out: &Value, inbound: &Value, pair_id: Option<&str>, policy: &str) -> Value {
        let rail = [out["asset"].as_str(), inbound["asset"].as_str()];
        json!({
            "pair_id": pair_id,
            "out_id": out["id"],
            "in_id": inbound["id"],
            "out_transaction_id": out["anchor"],
            "in_transaction_id": inbound["anchor"],
            "out_asset": out["asset"],
            "in_asset": inbound["asset"],
            "policy": policy,
            "bitcoin_rail": rail == [Some("BTC"), Some("LBTC")] || rail == [Some("LBTC"), Some("BTC")],
        })
    }

    fn rail(out: &Value, inbound: &Value, pair_id: Option<&str>) -> Value {
        cross(out, inbound, pair_id, "carrying-value")
    }

    /// A generic book under `method` (`null` for one the policy refuses,
    /// named `bogus`).
    fn book(
        method: Option<&str>,
        review: bool,
        rows: Vec<Value>,
        cross: Vec<Value>,
    ) -> LedgerRequest {
        serde_json::from_value(json!({
            "schema_version": 2,
            "operation": "ledger",
            "policy": {
                "country": {"kind": "generic", "long_term_days": 365},
                "method": method,
                "method_label": method.unwrap_or("bogus").to_uppercase(),
                "require_coarse_review": review,
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
            "intra_pairs": [],
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

    /// The `prepare` document, or the failure's.
    fn prepared(request: &LedgerRequest) -> Value {
        match prepare(request) {
            Ok(response) => serde_json::to_value(response).expect("serializes"),
            Err(failure) => serde_json::to_value(failure).expect("serializes"),
        }
    }

    /// The `(entry type, row, values)` of each entry, its values the ones
    /// an entry of its type has.
    fn entries(response: &Value) -> Vec<(String, String, Vec<String>)> {
        let text = |value: &Value| value.as_str().unwrap_or("null").to_owned();
        response["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| {
                let kind = entry["entry_type"].as_str().expect("type").to_owned();
                let row = entry["row"]
                    .as_str()
                    .unwrap_or("")
                    .replace("custody-tax:", "");
                let values = if kind == "acquisition" {
                    vec![text(&entry["quantity"]), text(&entry["fiat_value"])]
                } else {
                    ["quantity", "cost_basis", "proceeds", "gain_loss"]
                        .iter()
                        .map(|key| text(&entry[*key]))
                        .collect()
                };
                (kind, row, values)
            })
            .collect()
    }

    fn entry(kind: &str, row: &str, values: &[&str]) -> (String, String, Vec<String>) {
        (
            kind.to_owned(),
            row.to_owned(),
            values.iter().map(|value| (*value).to_owned()).collect(),
        )
    }

    /// The carried rows, by name.
    fn carried(response: &Value) -> Vec<(String, Value, Value)> {
        response
            .as_array()
            .expect("carried rows")
            .iter()
            .map(|item| {
                (
                    item["row"].as_str().unwrap().replace("custody-tax:", ""),
                    item["value"].clone(),
                    item["rate"].clone(),
                )
            })
            .collect()
    }

    fn carried_row(name: &str, value: &str, rate: Option<&str>) -> (String, Value, Value) {
        (name.to_owned(), json!(value), json!(rate))
    }

    /// The `(row, reason, detail)` of each quarantine.
    fn quarantines(response: &Value) -> Vec<(String, String, Value)> {
        response["quarantines"]
            .as_array()
            .expect("quarantines")
            .iter()
            .map(|item| {
                (
                    item["row"].as_str().unwrap().replace("custody-tax:", ""),
                    item["reason"].as_str().unwrap().to_owned(),
                    item["detail"].clone(),
                )
            })
            .collect()
    }

    fn rail_detail(outgoing: &str, incoming: &str, pair: &str, reason: &str) -> Value {
        json!({
            "outgoing_asset": outgoing,
            "incoming_asset": incoming,
            "rail_pair": pair,
            "policy": "carrying-value",
            "reason_code": reason,
        })
    }

    /// A purchase of 1 BTC, a 0.6 BTC peg into 0.599 LBTC, and a later
    /// Liquid sale, under the pair's `policy`.
    fn one_pair(policy: &str, peg_in: Value) -> LedgerRequest {
        let rows = vec![
            row(
                "buy",
                "C",
                "inbound",
                "2024-01-01T00:00:00Z",
                BTC,
                json!({}),
            ),
            row(
                "peg-out",
                "C",
                "outbound",
                "2024-05-01T00:00:00Z",
                60_000_000_000,
                json!({"rate": "60000E0"}),
            ),
            row(
                "peg-in",
                "L",
                "inbound",
                "2024-05-02T00:00:00Z",
                59_900_000_000,
                {
                    let mut fields = json!({"asset": "LBTC", "rate": "60000E0"});
                    for (key, value) in peg_in.as_object().expect("fields") {
                        fields[key] = value.clone();
                    }
                    fields
                },
            ),
            row(
                "liquid-sell",
                "L",
                "outbound",
                "2024-09-01T00:00:00Z",
                10_000_000_000,
                json!({"asset": "LBTC", "rate": "70000E0"}),
            ),
        ];
        let pair = cross(&rows[1], &rows[2], Some("rail-1"), policy);
        let review = peg_in.get("coarse") == Some(&json!(true));
        book(Some("fifo"), review, rows, vec![pair])
    }

    #[test]
    fn a_rail_pair_books_its_source_basis_on_both_legs() {
        let request = one_pair("carrying-value", json!({}));
        let response = run(&request);
        assert_eq!(
            carried(&response["carried_rows"]),
            vec![
                carried_row("peg-out", "24000", Some("40000")),
                carried_row("peg-in", "24000", Some("40066.777963272120200333889816361")),
            ]
        );
        assert_eq!(
            entries(&response),
            vec![
                entry("acquisition", "buy", &["1E0", "40000E0"]),
                entry(
                    "disposal",
                    "peg-out",
                    &["-6E-1", "240000E-1", "24000E0", "0E-1"]
                ),
                entry("acquisition", "peg-in", &["599E-3", "24000E0"]),
                entry(
                    "disposal",
                    "liquid-sell",
                    &[
                        "-1E-1",
                        "40066777963272120200333889816361E-28",
                        "70000E-1",
                        "29933222036727879799666110183639E-28"
                    ]
                ),
            ]
        );
        assert_eq!(response["quarantines"], json!([]));
        let carry = &prepared(&request)["rail_carry"];
        assert_eq!(carry["passes"], 2);
        assert_eq!(carry["carried"], response["carried_rows"]);
        assert_eq!(
            (&carry["blocked"], &carry["quarantines"]),
            (&json!([]), &json!([]))
        );
    }

    #[test]
    fn a_coarse_in_leg_the_carry_prices_is_not_reviewed() {
        // The carry prices it exactly: its first pass reviews it, the final
        // one books it at the carried basis.
        let response = run(&one_pair("carrying-value", json!({"coarse": true})));
        assert_eq!(response["quarantines"], json!([]));
        assert_eq!(
            entries(&response)[2],
            entry("acquisition", "peg-in", &["599E-3", "24000E0"])
        );
    }

    #[test]
    fn only_carrying_value_bitcoin_rail_pairs_carry() {
        // A taxable rail pair books at market.
        let request = one_pair("taxable", json!({}));
        let response = run(&request);
        assert_eq!(response["carried_rows"], json!([]));
        assert_eq!(
            entries(&response)[1..3],
            [
                entry(
                    "disposal",
                    "peg-out",
                    &["-6E-1", "240000E-1", "360000E-1", "120000E-1"]
                ),
                entry("acquisition", "peg-in", &["599E-3", "35940000E-3"]),
            ]
        );
        assert_eq!(prepared(&request)["rail_carry"]["passes"], 0);
        // So does a carrying-value pair into another asset on a generic
        // book (only Austria carries basis across a swap).
        let rows = vec![
            row(
                "buy",
                "C",
                "inbound",
                "2024-01-01T00:00:00Z",
                BTC,
                json!({}),
            ),
            row(
                "swap-out",
                "C",
                "outbound",
                "2024-05-01T00:00:00Z",
                50_000_000_000,
                json!({"rate": "60000E0"}),
            ),
            row(
                "swap-in",
                "C",
                "inbound",
                "2024-05-01T00:00:00Z",
                3_000_000_000_000_000,
                json!({"asset": "USDT", "rate": "1E0"}),
            ),
        ];
        let pair = rail(&rows[1], &rows[2], Some("swap-1"));
        let request = book(Some("fifo"), false, rows, vec![pair]);
        let response = run(&request);
        assert_eq!(response["carried_rows"], json!([]));
        assert_eq!(
            entries(&response),
            vec![
                entry("acquisition", "buy", &["1E0", "40000E0"]),
                entry(
                    "disposal",
                    "swap-out",
                    &["-5E-1", "200000E-1", "300000E-1", "100000E-1"]
                ),
                entry("acquisition", "swap-in", &["30000E0", "30000E0"]),
            ]
        );
        assert_eq!(prepared(&request)["rail_carry"]["passes"], 0);
    }

    /// Two buys, a peg into Liquid, and a peg back into another wallet.
    fn chain_rows(peg_in: Value) -> Vec<Value> {
        vec![
            row(
                "buy",
                "C",
                "inbound",
                "2024-01-01T00:00:00Z",
                BTC,
                json!({}),
            ),
            row(
                "buy-2",
                "C",
                "inbound",
                "2024-02-01T00:00:00Z",
                20_000_000_000,
                json!({"rate": "450005E-1"}),
            ),
            row(
                "peg-out",
                "C",
                "outbound",
                "2024-05-01T00:00:00Z",
                60_000_000_000,
                json!({"rate": "60000E0"}),
            ),
            row(
                "peg-in",
                "L",
                "inbound",
                "2024-05-02T00:00:00Z",
                60_000_000_000,
                {
                    let mut fields = json!({"asset": "LBTC", "rate": "60000E0"});
                    for (key, value) in peg_in.as_object().expect("fields") {
                        fields[key] = value.clone();
                    }
                    fields
                },
            ),
            row(
                "peg2-out",
                "L",
                "outbound",
                "2024-06-01T00:00:00Z",
                50_000_000_000,
                json!({"asset": "LBTC", "rate": "65000E0"}),
            ),
            row(
                "peg2-in",
                "A",
                "inbound",
                "2024-06-02T00:00:00Z",
                50_000_000_000,
                json!({"rate": "65000E0"}),
            ),
        ]
    }

    #[test]
    fn each_hop_carries_the_basis_the_last_hop_carried() {
        // The first pass prices hop 2 from the peg-in's market value
        // (30000), the second from its carried basis (20000), and the third
        // changes nothing.
        let rows = chain_rows(json!({}));
        let pairs = vec![
            rail(&rows[2], &rows[3], Some("rail-1")),
            rail(&rows[4], &rows[5], Some("rail-2")),
        ];
        let request = book(Some("fifo"), false, rows, pairs);
        let response = run(&request);
        assert_eq!(
            carried(&response["carried_rows"]),
            vec![
                carried_row("peg-out", "24000", Some("40000")),
                carried_row("peg-in", "24000", Some("40000")),
                carried_row("peg2-out", "20000", Some("40000")),
                carried_row("peg2-in", "20000", Some("40000")),
            ]
        );
        assert_eq!(
            entries(&response),
            vec![
                entry("acquisition", "buy", &["1E0", "40000E0"]),
                entry("acquisition", "buy-2", &["2E-1", "900010E-2"]),
                entry("acquisition", "peg2-in", &["5E-1", "20000E0"]),
                entry(
                    "disposal",
                    "peg-out",
                    &["-6E-1", "240000E-1", "24000E0", "0E-1"]
                ),
                entry("acquisition", "peg-in", &["6E-1", "24000E0"]),
                entry(
                    "disposal",
                    "peg2-out",
                    &["-5E-1", "20000E0", "20000E0", "0E0"]
                ),
            ]
        );
        assert_eq!(prepared(&request)["rail_carry"]["passes"], 3);
    }

    #[test]
    fn a_source_short_of_lots_blocks_its_pair_for_good() {
        // D-carry-1: the peg-in is unpriced, so on the first pass Liquid
        // holds nothing and hop 2 is short; the carry then prices the
        // peg-in, but the block stays, and the later sale from the wallet
        // hop 2 would have filled is short too.
        let mut rows = chain_rows(json!({"rate": null}));
        rows.remove(1);
        rows.push(row(
            "sell",
            "A",
            "outbound",
            "2024-07-01T00:00:00Z",
            10_000_000_000,
            json!({"rate": "70000E0"}),
        ));
        let pairs = vec![
            rail(&rows[1], &rows[2], Some("rail-1")),
            rail(&rows[3], &rows[4], Some("rail-2")),
        ];
        let request = book(Some("fifo"), false, rows, pairs);
        let response = run(&request);
        assert_eq!(
            carried(&response["carried_rows"]),
            vec![
                carried_row("peg-out", "24000", Some("40000")),
                carried_row("peg-in", "24000", Some("40000")),
            ]
        );
        let blocked = rail_detail("LBTC", "BTC", "rail-2", "insufficient_lots");
        assert_eq!(
            quarantines(&response),
            vec![
                (
                    "peg2-out".to_owned(),
                    "bitcoin_rail_carry_basis_unresolved".to_owned(),
                    blocked.clone()
                ),
                (
                    "peg2-in".to_owned(),
                    "bitcoin_rail_carry_basis_unresolved".to_owned(),
                    blocked
                ),
                (
                    "sell".to_owned(),
                    "insufficient_lots".to_owned(),
                    json!({"asset": "BTC", "available": {"float": "0E0"}, "required": {"float": "1E-1"}, "wallet": "Cold"})
                ),
            ]
        );
        assert_eq!(
            entries(&response),
            vec![
                entry("acquisition", "buy", &["1E0", "40000E0"]),
                entry(
                    "disposal",
                    "peg-out",
                    &["-6E-1", "240000E-1", "24000E0", "0E-1"]
                ),
                entry("acquisition", "peg-in", &["6E-1", "24000E0"]),
            ]
        );
        let prepared = prepared(&request);
        assert_eq!(prepared["rail_carry"]["passes"], 2);
        assert_eq!(
            prepared["rail_carry"]["blocked"],
            json!(["custody-tax:peg2-in", "custody-tax:peg2-out"])
        );
        // The final pass leaves the blocked rows out.
        let events: Vec<&str> = prepared["assets"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|asset| asset["normalized"]["events"].as_array().unwrap())
            .map(|event| event["row"].as_str().unwrap())
            .collect();
        assert!(!events.iter().any(|row| row.contains("peg2")), "{events:?}");
    }

    #[test]
    fn a_blocked_direct_payout_withholds_its_target_asset_disposal() {
        // The golden `carry-direct-payout-short-source`: the payout's
        // source is short of lots, so its pair is blocked for good. Both
        // settlement legs are quarantined, and the target-asset disposal
        // `<pair>:out` leaves the passes too, unquarantined: kept, it would
        // spend LBTC the blocked in leg never brought.
        let payout = |leg: &str, direction: &str, kind: &str| {
            row(
                "payout-source",
                "C",
                direction,
                "2025-03-01T09:00:30Z",
                49_990_000_000,
                json!({
                    "id": format!("direct-payout:payout-1:{leg}"),
                    "asset": "LBTC",
                    "rate": "50000E0",
                    "value": "24995E0",
                    "kind": kind,
                }),
            )
        };
        let rows = vec![
            row(
                "btc-buy",
                "C",
                "inbound",
                "2024-06-01T10:00:00Z",
                40_000_000_000,
                json!({"rate": "10000E0"}),
            ),
            row(
                "payout-source",
                "C",
                "outbound",
                "2025-03-01T09:00:00Z",
                50_000_000_000,
                json!({"kind": "withdrawal"}),
            ),
            payout("in", "inbound", "direct_swap_payout_in"),
            payout("out", "outbound", "sell"),
            row(
                "btc-sell",
                "C",
                "outbound",
                "2025-04-01T09:00:00Z",
                10_000_000_000,
                json!({"rate": "45000E0"}),
            ),
        ];
        let mut pair = rail(&rows[1], &rows[2], Some("direct-payout:payout-1"));
        pair["in_transaction_id"] = Value::Null;
        let request = book(Some("fifo"), false, rows, vec![pair]);
        let response = run(&request);
        let blocked = rail_detail("BTC", "LBTC", "direct-payout:payout-1", "insufficient_lots");
        let reason = "bitcoin_rail_carry_basis_unresolved".to_owned();
        assert_eq!(
            quarantines(&response),
            vec![
                ("payout-source".to_owned(), reason.clone(), blocked.clone()),
                ("direct-payout:payout-1:in".to_owned(), reason, blocked),
            ]
        );
        assert_eq!(response["carried_rows"], json!([]));
        assert_eq!(
            entries(&response),
            vec![
                entry("acquisition", "btc-buy", &["4E-1", "40000E-1"]),
                entry(
                    "disposal",
                    "btc-sell",
                    &["-1E-1", "10000E-1", "45000E-1", "35000E-1"]
                ),
            ]
        );
        let prepared = prepared(&request);
        assert_eq!(prepared["rail_carry"]["passes"], 2);
        assert_eq!(
            prepared["rail_carry"]["blocked"],
            json!([
                "custody-tax:payout-source",
                "direct-payout:payout-1:in",
                "direct-payout:payout-1:out",
            ])
        );
        // The final pass leaves all three out.
        let events: Vec<&str> = prepared["assets"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|asset| asset["normalized"]["events"].as_array().unwrap())
            .map(|event| event["row"].as_str().unwrap())
            .collect();
        assert!(
            !events
                .iter()
                .any(|row| row.contains("payout-source") || row.starts_with("direct-payout")),
            "{events:?}"
        );
    }

    #[test]
    fn a_pair_priced_and_then_blocked_keeps_its_pricing_and_its_block() {
        // Hop 1 is short in its source and blocked on pass 1, so its
        // Liquid receipt (booked at market there) leaves pass 2; hop 2,
        // priced from that receipt on pass 1, is short on pass 2.
        let rows = vec![
            row(
                "buy",
                "C",
                "inbound",
                "2024-01-01T00:00:00Z",
                BTC,
                json!({}),
            ),
            row(
                "peg-out",
                "C",
                "outbound",
                "2024-05-01T00:00:00Z",
                150_000_000_000,
                json!({"rate": "60000E0"}),
            ),
            row(
                "peg-in",
                "L",
                "inbound",
                "2024-05-02T00:00:00Z",
                150_000_000_000,
                json!({"asset": "LBTC", "rate": "60000E0"}),
            ),
            row(
                "peg2-out",
                "L",
                "outbound",
                "2024-06-01T00:00:00Z",
                50_000_000_000,
                json!({"asset": "LBTC", "rate": "65000E0"}),
            ),
            row(
                "peg2-in",
                "A",
                "inbound",
                "2024-06-02T00:00:00Z",
                50_000_000_000,
                json!({"rate": "65000E0"}),
            ),
            row(
                "sell",
                "C",
                "outbound",
                "2024-07-01T00:00:00Z",
                10_000_000_000,
                json!({"rate": "70000E0"}),
            ),
        ];
        let pairs = vec![
            rail(&rows[1], &rows[2], Some("rail-1")),
            rail(&rows[3], &rows[4], Some("rail-2")),
        ];
        let request = book(Some("fifo"), false, rows, pairs);
        let response = run(&request);
        assert_eq!(
            carried(&response["carried_rows"]),
            vec![
                carried_row("peg2-out", "30000", Some("60000")),
                carried_row("peg2-in", "30000", Some("60000")),
            ]
        );
        let first = rail_detail("BTC", "LBTC", "rail-1", "insufficient_lots");
        let second = rail_detail("LBTC", "BTC", "rail-2", "insufficient_lots");
        let reason = "bitcoin_rail_carry_basis_unresolved".to_owned();
        assert_eq!(
            quarantines(&response),
            vec![
                ("peg-out".to_owned(), reason.clone(), first.clone()),
                ("peg-in".to_owned(), reason.clone(), first),
                ("peg2-out".to_owned(), reason.clone(), second.clone()),
                ("peg2-in".to_owned(), reason, second),
            ]
        );
        assert_eq!(
            entries(&response),
            vec![
                entry("acquisition", "buy", &["1E0", "40000E0"]),
                entry(
                    "disposal",
                    "sell",
                    &["-1E-1", "40000E-1", "70000E-1", "30000E-1"]
                ),
            ]
        );
        assert_eq!(prepared(&request)["rail_carry"]["passes"], 3);
    }

    /// A same-instant round trip: BTC into Liquid and straight back.
    fn round_trip(method: &str) -> LedgerRequest {
        let at = "2024-05-01T00:00:00Z";
        let rows = vec![
            row(
                "buy",
                "C",
                "inbound",
                "2024-01-01T00:00:00Z",
                BTC,
                json!({}),
            ),
            row(
                "buy-2",
                "C",
                "inbound",
                "2024-02-01T00:00:00Z",
                BTC,
                json!({"rate": "50000E0"}),
            ),
            row(
                "peg-out",
                "C",
                "outbound",
                at,
                150_000_000_000,
                json!({"rate": "60000E0"}),
            ),
            row(
                "peg-in",
                "L",
                "inbound",
                at,
                150_000_000_000,
                json!({"asset": "LBTC", "rate": "60000E0"}),
            ),
            row(
                "peg2-out",
                "L",
                "outbound",
                at,
                150_000_000_000,
                json!({"asset": "LBTC", "rate": "60000E0"}),
            ),
            row(
                "peg2-in",
                "A",
                "inbound",
                at,
                150_000_000_000,
                json!({"rate": "60000E0"}),
            ),
            row(
                "sell",
                "A",
                "outbound",
                "2024-07-01T00:00:00Z",
                10_000_000_000,
                json!({"rate": "70000E0"}),
            ),
        ];
        let pairs = vec![
            rail(&rows[2], &rows[3], Some("rail-1")),
            rail(&rows[4], &rows[5], Some("rail-2")),
        ];
        book(Some(method), false, rows, pairs)
    }

    #[test]
    fn carries_that_do_not_settle_keep_the_last_passs_pricing() {
        // D-carry-2: under the moving average each pass moves the basis
        // round the trip again; the passes run out, and the LBTC peg-out
        // books a gain on a carrying-value leg.
        let request = round_trip("moving_average");
        let response = run(&request);
        assert_eq!(
            carried(&response["carried_rows"]),
            vec![
                carried_row(
                    "peg-out",
                    "71632.65306122448979591836734694",
                    Some("47755.10204081632653061224489796")
                ),
                carried_row(
                    "peg-in",
                    "71632.65306122448979591836734694",
                    Some("47755.10204081632653061224489796")
                ),
                carried_row(
                    "peg2-out",
                    "77142.85714285714285714285714286",
                    Some("51428.571428571428571428571428573")
                ),
                carried_row(
                    "peg2-in",
                    "77142.85714285714285714285714286",
                    Some("51428.571428571428571428571428573")
                ),
            ]
        );
        let gains: Vec<(String, String)> = entries(&response)
            .into_iter()
            .filter(|(kind, ..)| kind == "disposal")
            .map(|(_, row, values)| (row, values[3].clone()))
            .collect();
        assert_eq!(
            gains,
            vec![
                ("peg-out".to_owned(), "0E-27".to_owned()),
                (
                    "sell".to_owned(),
                    "2224489795918367346938775510204E-27".to_owned()
                ),
                (
                    "peg2-out".to_owned(),
                    "5510204081632653061224489795920E-27".to_owned()
                ),
            ]
        );
        assert_eq!(prepared(&request)["rail_carry"]["passes"], 3);
        // D-carry-3: under FIFO the same trip settles; both legs gain
        // nothing.
        let response = run(&round_trip("fifo"));
        let carried_values: Vec<Value> = response["carried_rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["value"].clone())
            .collect();
        assert_eq!(carried_values, vec![json!("65000"); 4]);
        let gains: Vec<String> = entries(&response)
            .into_iter()
            .filter(|(kind, row, _)| kind == "disposal" && row.starts_with("peg"))
            .map(|(.., values)| values[3].clone())
            .collect();
        assert_eq!(gains, vec!["0E-27", "0E0"]);
    }

    #[test]
    fn the_method_gate_fails_the_first_carry_pass() {
        let mut request = one_pair("carrying-value", json!({}));
        request.policy.method = None;
        request.policy.method_label = "bogus".to_owned();
        let response = run(&request);
        assert_eq!(
            response["error"],
            json!({
                "stage": "compute_multi",
                "inner": {"class": "AppError", "message": "Unsupported RP2 accounting method 'bogus'"},
            })
        );
        // `prepare` lists that pass's stages, every asset's, and no carry.
        let prepared = prepared(&request);
        assert_eq!(prepared["error"], response["error"]);
        assert_eq!(prepared["rail_carry"], Value::Null);
        assert_eq!(prepared["swap_links"], Value::Null);
        let assets = prepared["assets"].as_array().unwrap();
        assert_eq!(assets.len(), 2);
        assert!(assets.iter().all(|asset| asset["prepared"].is_object()));
    }

    #[test]
    fn a_source_without_basis_or_a_reason_blocks_its_pair_as_unavailable() {
        // The pair's out row is a purchase: no fragment basis, and no
        // quarantine names it. The in leg's missing price does not explain
        // it, as the carry would price that leg.
        let rows = vec![
            row(
                "buy",
                "C",
                "inbound",
                "2024-01-01T00:00:00Z",
                BTC,
                json!({}),
            ),
            row(
                "peg-in",
                "L",
                "inbound",
                "2024-05-02T00:00:00Z",
                50_000_000_000,
                json!({"asset": "LBTC", "rate": null}),
            ),
            row(
                "sell",
                "C",
                "outbound",
                "2024-07-01T00:00:00Z",
                10_000_000_000,
                json!({"rate": "70000E0"}),
            ),
        ];
        let pair = rail(&rows[0], &rows[1], None);
        let request = book(Some("fifo"), false, rows, vec![pair]);
        let response = run(&request);
        let blocked = rail_detail(
            "BTC",
            "LBTC",
            "custody-tax:buy->custody-tax:peg-in",
            "source_basis_unavailable",
        );
        let reason = "bitcoin_rail_carry_basis_unresolved".to_owned();
        assert_eq!(
            quarantines(&response),
            vec![
                ("buy".to_owned(), reason.clone(), blocked.clone()),
                ("peg-in".to_owned(), reason, blocked),
                (
                    "sell".to_owned(),
                    "insufficient_lots".to_owned(),
                    json!({"asset": "BTC", "available": {"float": "0E0"}, "required": {"float": "1E-1"}, "wallet": "Exchange"})
                ),
            ]
        );
        assert_eq!(response["carried_rows"], json!([]));
        assert_eq!(prepared(&request)["rail_carry"]["passes"], 2);
    }

    #[test]
    fn a_source_without_an_amount_carries_no_rate() {
        // A fee-only peg-out: its fee's basis is carried, and only the in
        // leg has an amount to price it by.
        let rows = vec![
            row(
                "buy",
                "C",
                "inbound",
                "2024-01-01T00:00:00Z",
                BTC,
                json!({}),
            ),
            row(
                "peg-out",
                "C",
                "outbound",
                "2024-05-01T00:00:00Z",
                0,
                json!({"rate": "60000E0", "fee_msat": 1_000_000}),
            ),
            row(
                "peg-in",
                "L",
                "inbound",
                "2024-05-02T00:00:00Z",
                500_000,
                json!({"asset": "LBTC", "rate": "60000E0"}),
            ),
        ];
        let pair = rail(&rows[1], &rows[2], Some("rail-1"));
        let response = run(&book(Some("fifo"), false, rows, vec![pair]));
        assert_eq!(
            carried(&response["carried_rows"]),
            vec![
                carried_row("peg-out", "0.4", None),
                carried_row("peg-in", "0.4", Some("80000")),
            ]
        );
        assert_eq!(
            entries(&response),
            vec![
                entry("acquisition", "buy", &["1E0", "40000E0"]),
                entry("fee", "peg-out", &["-1E-5", "40000E-5", "40000E-5", "0E0"]),
                entry("acquisition", "peg-in", &["5E-6", "4E-1"]),
            ]
        );
    }
}
