//! Properties of the `ledger` operation on seeded generic and Austrian
//! books (plan 20 Verification 1, design §4.12).
//!
//! A small xorshift generator writes schema-2 requests a finalized custody
//! projection can produce (design §3.5): synthetic row ids, labelled
//! wallets, purchases (some unpriced, some coarse), income of listed and
//! unlisted kinds, sales (some larger than the wallet holds, some fee-only,
//! some gifts), moves between wallets (ungrouped, in their own bundle, or
//! in a bundle with the previous move; priced on one leg, both, or none),
//! held rows and privacy hops, over BTC and sometimes LBTC, with every
//! generic accounting method. On such books:
//!
//! 1. the response is a pure function of the request: two runs give the
//!    same bytes;
//! 2. msat are conserved per asset: what the acquisitions bring in, less
//!    what disposals, fees, and move fees take out, is what the holdings
//!    hold; and each audited move sends what it receives plus its fee;
//! 3. the mirror holds: every book succeeds, so no account ever went
//!    negative in the lot engine's balance check, and replaying the
//!    journal per wallet ends at each wallet's holding, never below zero
//!    (a generic book has no swap-out that may skip the gates);
//! 4. pricing a previously unpriced inbound row (a purchase, income, or a
//!    move's in leg) never quarantines a row that was not quarantined
//!    before, with one explicit exclusion. A quarantined disposal consumes
//!    nothing, so pricing can release a sale or move the missing price had
//!    held (short, or after the tainted lot state), which then leaves a
//!    later move or sale short. The oracle does the same (seeds 156 and 168
//!    here, replayed on it; the goldens `generic-pricing-releases-a-sale-*`
//!    pin the smallest such book), so it is kept until the phase-5
//!    inventory change (TODO, "Quarantine follow-ups"): a shortfall gate at
//!    or after the time of a released outbound row is excused, and nothing
//!    else is.
//!
//! Austrian books start shortly before the Neuvermögen cutoff, so they hold
//! both regimes, and carry reviewed regime, category, and taxability
//! overrides, over every method Austria's policy allows. On them the same
//! properties hold, except that a book may end in the Neu shortfall abort the
//! oracle shares (design §6), rarely, and pricing may release one; and:
//!
//! 5. every entry an Austrian event or move fee books starts with its
//!    regime and names the pool, an overridden row's regime is the
//!    override's, and each audited move's regime flows conserve (what
//!    reached the destination left the source, per regime, and the
//!    difference is the fee, or all that left when that was less) and are
//!    what its legs' descriptions say;
//! 6. categories follow the reviewed overrides: a fee reports none, and a
//!    disposal or income reports the override's, none when the override
//!    clears it, and the lot engine's otherwise.
//!
//! Basis conservation across carries is the carries' property (Austrian
//! swaps and the generic rail carry, not yet on the engine).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use kassiber_tax_core::decimal::Decimal;
use kassiber_tax_core::ledger;
use kassiber_tax_core::time::CivilDate;
use serde_json::{json, Value};

/// Seeds per property.
const SEEDS: u64 = 300;

/// xorshift64* (Vigna), seeded away from zero.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

const WALLETS: [(&str, &str); 3] = [("A", "Cold"), ("B", "Hot"), ("C", "Exchange")];
const METHODS: [(&str, &str); 5] = [
    ("fifo", "FIFO"),
    ("lifo", "LIFO"),
    ("hifo", "HIFO"),
    ("lofo", "LOFO"),
    ("moving_average", "MOVING_AVERAGE"),
];

/// Austria's methods: its own three, as profiles spell them, and the
/// generic lot methods its policy allows too (`build_austrian_policy`).
/// The default, `moving_average_at`, is listed twice: only it splits lots
/// by regime, so only it can end in the Neu shortfall, and this keeps that
/// exclusion in use at `SEEDS`.
const AUSTRIAN_METHODS: [(&str, &str); 7] = [
    ("moving_average_at", "moving_average_at"),
    ("moving_average_at", "moving_average_at"),
    ("fifo", "fifo"),
    ("moving_average", "moving_average"),
    ("hifo", "hifo"),
    ("lifo", "lifo"),
    ("lofo", "lofo"),
];

/// `AT_NEU_CUTOFF`, 2021-03-01 00:00 CET, in seconds since the epoch.
const CUTOFF_S: i64 = 1_614_553_200;

/// A parsed UTC timestamp `seconds` after the epoch.
fn ts(seconds: i64) -> Value {
    let date = CivilDate::from_days(seconds.div_euclid(86_400));
    let day = seconds.rem_euclid(86_400);
    let date = format!("{:04}-{:02}-{:02}", date.year, date.month, date.day);
    let time = format!("{:02}:{:02}:{:02}", day / 3600, day / 60 % 60, day % 60);
    json!({
        "raw": format!("{date}T{time}Z"),
        "us": seconds * 1_000_000,
        "offset_s": 0,
        "date": date,
        "year": date[..4].parse::<i32>().unwrap(),
        "vienna_date": date,
        "display": format!("{date} {time}+00:00"),
        "iso": format!("{date}T{time}+00:00"),
    })
}

/// A price per BTC in canonical form, with up to two decimal places.
fn price(rng: &mut Rng) -> String {
    let cents = 100_000 + rng.below(9_000_000);
    format!("{cents}E-2")
}

struct Builder {
    rng: Rng,
    /// An Austrian book: rows around the Neuvermögen cutoff, with regime,
    /// category, and taxability overrides. A generic book draws nothing
    /// for these, so its stream of choices is the same as before.
    austrian: bool,
    rows: Vec<Value>,
    pairs: Vec<Value>,
    /// The next row's number.
    next_id: usize,
    /// Seconds since the epoch of the next row.
    clock: i64,
    /// A rough balance per (asset, wallet), so sales and moves are mostly
    /// in range.
    held: HashMap<(&'static str, &'static str), u64>,
    /// The previous move's bundle, which the next move may join.
    last_group: Option<String>,
}

impl Builder {
    fn new(seed: u64, austrian: bool) -> Self {
        let mut rng = Rng::new(seed);
        // An Austrian book starts up to 90 days before the cutoff
        // (2021-03-01 00:00 CET), so its history spans both regimes.
        let clock = if austrian {
            CUTOFF_S - rng.below(90 * 86_400) as i64
        } else {
            1_672_531_200 // 2023-01-01
        };
        Builder {
            rng,
            austrian,
            rows: Vec::new(),
            pairs: Vec::new(),
            next_id: 0,
            clock,
            held: HashMap::new(),
            last_group: None,
        }
    }

    fn tick(&mut self) {
        // Same instant, minutes, hours, or days later.
        self.clock += *self.rng.pick(&[0, 600, 7_200, 86_400, 259_200]);
    }

    fn row(
        &mut self,
        asset: &'static str,
        wallet: &'static str,
        direction: &str,
        amount: u64,
    ) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let priced = !self.rng.chance(8);
        let mut row = json!({
            "id": format!("custody-tax:{id:04}"),
            "anchor": format!("tx-{id:04}"),
            "asset": asset,
            "wallet_id": wallet,
            "direction": direction,
            "ts": ts(self.clock),
            "cutoff_instant": {"us": self.clock * 1_000_000},
            "amount_msat": amount,
            "fee_msat": 0,
            "rate": if priced { Value::from(price(&mut self.rng)) } else { Value::Null },
            "value": null,
            "coarse": priced && self.rng.chance(10),
            "kind": if direction == "inbound" { "buy" } else { "sell" },
            "text": if self.rng.chance(70) { Value::from(format!("row {id}")) } else { Value::Null },
            "regime_override": null,
            "category_override": null,
            "taxability_off": false,
            "privacy_hop": self.rng.chance(2),
            "hold": match self.rng.below(100) {
                0 | 1 => Value::from("pending_onchain_confirmation"),
                2 => Value::from("conflicting_spend"),
                _ => Value::Null,
            },
            "chain_order": null,
        });
        if self.austrian {
            self.overrides(&mut row);
        }
        row
    }

    /// An Austrian row's reviewed overrides, now and then.
    fn overrides(&mut self, row: &mut Value) {
        if self.rng.chance(8) {
            row["regime_override"] = json!(self.rng.pick(&["alt", "neu"]));
        }
        if self.rng.chance(6) {
            row["category_override"] = json!(self.rng.pick(&[
                "none",
                "neu_swap",
                "alt_taxfree",
                "neu_gain",
                "alt_spekulation"
            ]));
        }
        if self.rng.chance(4) {
            row["taxability_off"] = json!(true);
        }
    }

    fn amount(&mut self, held: u64) -> u64 {
        // Mostly within what the wallet holds, sometimes beyond it.
        let cap = if held > 0 && !self.rng.chance(10) {
            held
        } else {
            50_000_000_000
        };
        1 + self.rng.below(cap.max(2))
    }

    fn step(&mut self, asset: &'static str) {
        self.tick();
        let (wallet, _) = *self.rng.pick(&WALLETS);
        let held = *self.held.get(&(asset, wallet)).unwrap_or(&0);
        match self.rng.below(100) {
            0..=34 => {
                let amount = 1_000_000 + self.rng.below(200_000_000_000);
                let row = self.row(asset, wallet, "inbound", amount);
                *self.held.entry((asset, wallet)).or_default() += amount;
                self.rows.push(row);
            }
            35..=44 => {
                let amount = 1_000_000 + self.rng.below(5_000_000_000);
                let mut row = self.row(asset, wallet, "inbound", amount);
                row["kind"] = json!(self.rng.pick(&[
                    "staking",
                    "income",
                    "mining",
                    "interest",
                    "airdrop",
                    "wages",
                    "referral_bonus",
                    "cashback"
                ]));
                *self.held.entry((asset, wallet)).or_default() += amount;
                self.rows.push(row);
            }
            45..=74 => {
                let amount = self.amount(held);
                let fee = if self.rng.chance(40) {
                    1 + self.rng.below(50_000_000)
                } else {
                    0
                };
                let fee_only = self.rng.chance(6);
                let mut row =
                    self.row(asset, wallet, "outbound", if fee_only { 0 } else { amount });
                row["fee_msat"] = json!(if fee_only { fee.max(1) } else { fee });
                if self.rng.chance(5) {
                    row["kind"] = json!(self.rng.pick(&["gift", "lost", "donation"]));
                }
                let spent = if fee_only { fee.max(1) } else { amount + fee };
                let slot = self.held.entry((asset, wallet)).or_default();
                *slot = slot.saturating_sub(spent);
                self.rows.push(row);
            }
            _ => self.transfer(asset, wallet, held),
        }
    }

    fn transfer(&mut self, asset: &'static str, from: &'static str, held: u64) {
        let (to, _) = *self.rng.pick(&WALLETS);
        if to == from {
            return;
        }
        let amount = self.amount(held);
        let fee = if self.rng.chance(50) {
            1 + self.rng.below(20_000_000)
        } else {
            0
        };
        let mut out = self.row(asset, from, "outbound", amount);
        out["fee_msat"] = json!(fee);
        out["kind"] = json!("transfer");
        let shared_anchor = self.rng.chance(50);
        if self.rng.chance(30) {
            self.clock += 600;
        }
        let mut inbound = self.row(asset, to, "inbound", amount);
        inbound["kind"] = json!("transfer");
        inbound["privacy_hop"] = json!(false);
        out["privacy_hop"] = json!(false);
        if shared_anchor {
            inbound["anchor"] = out["anchor"].clone();
        }
        match self.rng.below(4) {
            0 => inbound["rate"] = Value::Null,
            1 => out["rate"] = Value::Null,
            _ => {}
        }
        let group = match self.rng.below(10) {
            0..=3 => None,
            4..=6 => self.last_group.clone(),
            _ => Some(format!("bundle:{}", self.pairs.len())),
        };
        self.last_group = group
            .clone()
            .or_else(|| Some(format!("bundle:{}", self.pairs.len())));
        self.pairs.push(json!({
            "out": out["id"],
            "in": inbound["id"],
            "source": if self.rng.chance(50) { Value::from("reviewed_custody_component") } else { Value::Null },
            "group_id": group,
        }));
        let slot = self.held.entry((asset, from)).or_default();
        *slot = slot.saturating_sub(amount + fee);
        *self.held.entry((asset, to)).or_default() += amount;
        self.rows.push(out);
        self.rows.push(inbound);
    }
}

/// A generic book for `seed`.
fn book(seed: u64) -> Value {
    book_of(seed, false)
}

/// An Austrian book for `seed`.
fn austrian_book(seed: u64) -> Value {
    book_of(seed, true)
}

fn book_of(seed: u64, austrian: bool) -> Value {
    let mut builder = Builder::new(seed, austrian);
    let steps = 3 + builder.rng.below(30);
    let liquid = builder.rng.chance(30);
    for _ in 0..steps {
        let asset = if liquid && builder.rng.chance(35) {
            "LBTC"
        } else {
            "BTC"
        };
        builder.step(asset);
    }
    let (method, label) = if austrian {
        *builder.rng.pick(&AUSTRIAN_METHODS)
    } else {
        *builder.rng.pick(&METHODS)
    };
    let country = if austrian {
        json!({"kind": "at"})
    } else {
        json!({"kind": "generic", "long_term_days": *builder.rng.pick(&[365_u64, 30, 0])})
    };
    let require_coarse_review = builder.rng.chance(30);
    let mut rows = builder.rows;
    // The projection lists its rows by time, then id.
    rows.sort_by(|a, b| {
        (a["ts"]["raw"].as_str(), a["id"].as_str())
            .cmp(&(b["ts"]["raw"].as_str(), b["id"].as_str()))
    });
    json!({
        "schema_version": 2,
        "operation": "ledger",
        "policy": {
            "country": country,
            "method": method,
            "method_label": label,
            "require_coarse_review": require_coarse_review,
        },
        "config": {
            "assets": ["BTC", "LBTC"],
            "exchanges": ["Cold", "Exchange", "Hot"],
            "holders": ["Default"],
        },
        "holder": "Default",
        "wallets": WALLETS.iter().map(|(id, label)| json!({"id": id, "label": label})).collect::<Vec<_>>(),
        "rows": rows,
        "intra_pairs": builder.pairs,
        "cross_asset_pairs": [],
    })
}

fn run(request: &Value) -> Value {
    serde_json::from_str(&ledger(&request.to_string())).expect("the response is JSON")
}

/// A canonical decimal in msat; every quantity here is a whole number of
/// msat.
fn msat(text: &Value) -> i128 {
    let value = Decimal::from_canonical(text.as_str().expect("a decimal")).expect("canonical");
    // BTC to msat: eleven places to the left.
    let exponent = value.exponent() + 11;
    let mut digits = value.coefficient().to_string();
    if exponent >= 0 {
        digits.push_str(&"0".repeat(exponent as usize));
    } else {
        let keep = digits
            .len()
            .saturating_sub(exponent.unsigned_abs() as usize);
        assert!(
            digits[keep..].bytes().all(|b| b == b'0'),
            "{text} is not whole msat"
        );
        digits.truncate(keep);
    }
    let scaled: i128 = if digits.is_empty() {
        0
    } else {
        digits.parse().expect("msat")
    };
    if value.is_negative() {
        -scaled
    } else {
        scaled
    }
}

/// Each row's (asset, wallet label), by id.
fn row_accounts(request: &Value) -> HashMap<String, (String, String)> {
    let labels: HashMap<&str, &str> = WALLETS.iter().copied().collect();
    request["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["id"].as_str().unwrap().to_owned(),
                (
                    row["asset"].as_str().unwrap().to_owned(),
                    labels[row["wallet_id"].as_str().unwrap()].to_owned(),
                ),
            )
        })
        .collect()
}

#[test]
fn the_same_request_gives_the_same_bytes() {
    for seed in 0..SEEDS {
        let text = book(seed).to_string();
        assert_eq!(ledger(&text), ledger(&text), "seed {seed}");
    }
}

#[test]
fn msat_are_conserved_per_asset_and_per_move() {
    let mut moves = 0;
    let mut realized = 0;
    for seed in 0..SEEDS {
        let request = book(seed);
        let response = run(&request);
        assert_eq!(response["ok"], true, "seed {seed}: {response}");
        let accounts = row_accounts(&request);
        let mut flow: BTreeMap<String, i128> = BTreeMap::new();
        for entry in response["entries"].as_array().unwrap() {
            let kind = entry["entry_type"].as_str().unwrap();
            if matches!(kind, "acquisition" | "disposal" | "fee" | "transfer_fee") {
                let asset = accounts[entry["row"].as_str().unwrap()].0.clone();
                *flow.entry(asset).or_default() += msat(&entry["quantity"]);
                realized += usize::from(kind != "acquisition");
            }
        }
        let mut held: BTreeMap<String, i128> = BTreeMap::new();
        for holding in response["holdings"].as_array().unwrap() {
            *held
                .entry(holding["asset"].as_str().unwrap().to_owned())
                .or_default() += msat(&holding["quantity"]);
        }
        flow.retain(|_, value| *value != 0);
        assert_eq!(flow, held, "seed {seed}");
        for audit in response["intra_audit"].as_array().unwrap() {
            let (sent, received, fee) = (
                audit["sent_msat"].as_u64().unwrap(),
                audit["received_msat"].as_u64().unwrap(),
                audit["fee_msat"].as_u64().unwrap(),
            );
            assert_eq!(sent, received + fee, "seed {seed}");
            assert_eq!(msat(&audit["sent"]), i128::from(sent), "seed {seed}");
            assert_eq!(
                msat(&audit["received"]),
                i128::from(received),
                "seed {seed}"
            );
            assert_eq!(msat(&audit["fee"]), i128::from(fee), "seed {seed}");
            moves += 1;
        }
    }
    // The generator reaches moves and realized entries.
    assert!(moves > SEEDS as usize, "only {moves} audited moves");
    assert!(
        realized > SEEDS as usize,
        "only {realized} realized entries"
    );
}

#[test]
fn every_wallet_ends_at_its_holding_and_never_below_zero() {
    let mut quarantined = 0;
    for seed in 0..SEEDS {
        let request = book(seed);
        let response = run(&request);
        assert_eq!(response["ok"], true, "seed {seed}: {response}");
        quarantined += response["quarantines"].as_array().unwrap().len();
        let accounts = row_accounts(&request);
        let pairs = request["intra_pairs"].as_array().unwrap();
        let audit = response["intra_audit"].as_array().unwrap();
        let mut balance: BTreeMap<(String, String), i128> = BTreeMap::new();
        for entry in response["entries"].as_array().unwrap() {
            let kind = entry["entry_type"].as_str().unwrap();
            let (account, quantity) = match kind {
                "acquisition" | "disposal" | "fee" => (
                    accounts[entry["row"].as_str().unwrap()].clone(),
                    msat(&entry["quantity"]),
                ),
                "transfer_out" | "transfer_in" => {
                    let row = &audit[entry["audit"].as_u64().unwrap() as usize];
                    let pair = &pairs[row["pair"].as_u64().unwrap() as usize];
                    if kind == "transfer_out" {
                        (
                            accounts[pair["out"].as_str().unwrap()].clone(),
                            -msat(&row["sent"]),
                        )
                    } else {
                        (
                            accounts[pair["in"].as_str().unwrap()].clone(),
                            msat(&row["received"]),
                        )
                    }
                }
                // Income is the acquisition's; a move's fee is in what it sent.
                _ => continue,
            };
            *balance.entry(account).or_default() += quantity;
        }
        let mut held: BTreeMap<(String, String), i128> = BTreeMap::new();
        for holding in response["holdings"].as_array().unwrap() {
            let account = (
                holding["asset"].as_str().unwrap().to_owned(),
                holding["wallet"].as_str().unwrap().to_owned(),
            );
            held.insert(account, msat(&holding["quantity"]));
        }
        for (account, value) in &balance {
            assert!(*value >= 0, "seed {seed}: {account:?} ends at {value} msat");
        }
        balance.retain(|_, value| *value != 0);
        assert_eq!(balance, held, "seed {seed}");
    }
    assert!(
        quarantined > SEEDS as usize,
        "the gates quarantine only {quarantined} rows"
    );
}

/// Each quarantined row with its first reason.
fn quarantined_rows(response: &Value) -> BTreeMap<String, String> {
    let mut rows = BTreeMap::new();
    for item in response["quarantines"].as_array().unwrap() {
        rows.entry(item["row"].as_str().unwrap().to_owned())
            .or_insert_with(|| item["reason"].as_str().unwrap().to_owned());
    }
    rows
}

/// The rows pricing quarantined that were not quarantined before, less the
/// ones the exclusion of property 4 excuses: a shortfall gate (or the group
/// flag it raises) at or after the time of an outbound row that pricing
/// released.
fn unexcused_additions(request: &Value, before: &Value, after: &Value) -> (Vec<String>, usize) {
    let rows: HashMap<&str, &Value> = request["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["id"].as_str().unwrap(), row))
        .collect();
    let time = |id: &str| rows[id]["ts"]["raw"].as_str().unwrap().to_owned();
    let (before, after) = (quarantined_rows(before), quarantined_rows(after));
    let released: Vec<String> = before
        .keys()
        .filter(|id| !after.contains_key(*id) && rows[id.as_str()]["direction"] == "outbound")
        .map(|id| time(id))
        .collect();
    let mut excused = 0;
    let mut unexcused = Vec::new();
    for (id, reason) in &after {
        if before.contains_key(id) {
            continue;
        }
        let shortfall = matches!(
            reason.as_str(),
            "insufficient_lots" | "missing_cost_basis" | "derived_transfer_group_blocked"
        );
        if shortfall && released.iter().any(|since| *since <= time(id)) {
            excused += 1;
        } else {
            unexcused.push(format!("{id} ({reason})"));
        }
    }
    (unexcused, excused)
}

#[test]
fn pricing_an_unpriced_inbound_row_quarantines_no_new_row() {
    let mut repriced = 0;
    let mut repriced_legs = 0;
    let mut excused = 0;
    for seed in 0..SEEDS {
        let request = book(seed);
        let in_legs: BTreeSet<&str> = request["intra_pairs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| pair["in"].as_str().unwrap())
            .collect();
        let rows = request["rows"].as_array().unwrap();
        let unpriced: Vec<usize> = (0..rows.len())
            .filter(|&index| {
                let row = &rows[index];
                row["direction"] == "inbound" && row["rate"].is_null() && row["value"].is_null()
            })
            .collect();
        let before = run(&request);
        assert_eq!(before["ok"], true, "seed {seed}");
        for index in unpriced {
            let mut priced = request.clone();
            priced["rows"][index]["rate"] = json!("4000000E-2");
            priced["rows"][index]["coarse"] = json!(false);
            let after = run(&priced);
            assert_eq!(after["ok"], true, "seed {seed}, row {index}: {after}");
            let (added, released) = unexcused_additions(&request, &before, &after);
            assert!(
                added.is_empty(),
                "seed {seed}: pricing row {index} quarantines {added:?}"
            );
            excused += released;
            repriced += 1;
            repriced_legs += usize::from(in_legs.contains(rows[index]["id"].as_str().unwrap()));
        }
    }
    assert!(
        repriced > SEEDS as usize / 2,
        "only {repriced} inbound rows repriced"
    );
    assert!(
        repriced_legs > SEEDS as usize / 4,
        "only {repriced_legs} move in legs repriced"
    );
    // The exclusion is used, and rarely.
    assert!(
        (1..=SEEDS as usize / 20).contains(&excused),
        "{excused} rows excused"
    );
}

#[test]
fn pricing_a_purchase_can_release_a_sale_that_starves_a_later_move() {
    // The smallest book of the exclusion, as the goldens
    // `generic-pricing-releases-a-sale-*` store the oracle's outcome.
    let row = |id: &str, wallet: &str, direction: &str, day: i64, amount: u64, rate: Value| {
        json!({
            "id": format!("custody-tax:{id}"),
            "anchor": id.trim_end_matches("-out").trim_end_matches("-in"),
            "asset": "BTC",
            "wallet_id": wallet,
            "direction": direction,
            "ts": ts(1_704_067_200 + day * 86_400),
            "cutoff_instant": {"us": (1_704_067_200 + day * 86_400) * 1_000_000},
            "amount_msat": amount,
            "fee_msat": 0,
            "rate": rate,
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
        })
    };
    let request = |late_rate: Value| {
        let mut request = book(0);
        request["policy"]["method"] = json!("fifo");
        request["policy"]["require_coarse_review"] = json!(false);
        request["rows"] = json!([
            row("buy", "A", "inbound", 0, 100_000_000_000, json!("40000E0")),
            row("late-buy", "A", "inbound", 31, 100_000_000_000, late_rate),
            row(
                "sale",
                "A",
                "outbound",
                60,
                150_000_000_000,
                json!("50000E0")
            ),
            row(
                "move-out",
                "A",
                "outbound",
                91,
                80_000_000_000,
                json!("50000E0")
            ),
            row(
                "move-in",
                "B",
                "inbound",
                91,
                80_000_000_000,
                json!("50000E0")
            ),
        ]);
        request["intra_pairs"] = json!([{
            "out": "custody-tax:move-out",
            "in": "custody-tax:move-in",
            "source": "reviewed_custody_component",
            "group_id": null,
        }]);
        request
    };
    let outcome = |response: &Value| -> (Vec<(String, String)>, Vec<String>) {
        (
            quarantined_rows(response).into_iter().collect(),
            response["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["entry_type"].as_str().unwrap().to_owned())
                .collect(),
        )
    };
    let unpriced = request(Value::Null);
    let before = run(&unpriced);
    let after = run(&request(json!("40000E0")));
    let pair = |row: &str, reason: &str| (format!("custody-tax:{row}"), reason.to_owned());
    assert_eq!(
        outcome(&before),
        (
            vec![
                pair("late-buy", "missing_spot_price"),
                pair("sale", "insufficient_lots")
            ],
            vec![
                "acquisition".to_owned(),
                "transfer_out".to_owned(),
                "transfer_in".to_owned()
            ],
        )
    );
    assert_eq!(
        outcome(&after),
        (
            vec![pair("move-out", "insufficient_lots")],
            vec![
                "acquisition".to_owned(),
                "acquisition".to_owned(),
                "disposal".to_owned()
            ],
        )
    );
    assert_eq!(
        unexcused_additions(&unpriced, &before, &after),
        (Vec::new(), 1)
    );
}

// Austrian books (design §4.12, WI-9): the same properties, and the
// regimes, flows, and categories the Austrian rules add. An Austrian book
// can fail where the oracle fails: a Neu disposal larger than the Neu pool
// holds aborts the computation (the Neu shortfall, design §6); every other
// book must succeed.

/// The Neu shortfall abort: the lot engine's moving-average pool holds less
/// than a Neu disposal takes (`tests/tax_oracle/adapter.py` raises it as
/// `RP2 tax calculation failed for asset ...`).
fn is_neu_shortfall(response: &Value) -> bool {
    let error = &response["error"];
    error["stage"] == "compute"
        && error["inner"]["class"] == "ValueError"
        && error["inner"]["message"]
            == "Total in-transaction crypto value < total taxable crypto value"
}

/// The Austrian books that succeed, with their responses; the others must
/// fail with the Neu shortfall, and rarely.
fn austrian_outcomes() -> Vec<(u64, Value, Value)> {
    let mut books = Vec::new();
    let mut shortfalls = 0;
    for seed in 0..SEEDS {
        let request = austrian_book(seed);
        let response = run(&request);
        if response["ok"] == true {
            books.push((seed, request, response));
        } else {
            assert!(is_neu_shortfall(&response), "seed {seed}: {response}");
            shortfalls += 1;
        }
    }
    assert!(
        shortfalls <= SEEDS as usize / 50,
        "{shortfalls} Neu shortfalls"
    );
    books
}

#[test]
fn austrian_books_give_the_same_bytes() {
    for seed in 0..SEEDS {
        let text = austrian_book(seed).to_string();
        assert_eq!(ledger(&text), ledger(&text), "seed {seed}");
    }
}

#[test]
fn austrian_msat_are_conserved_and_wallets_end_at_their_holdings() {
    let mut moves = 0;
    for (seed, request, response) in austrian_outcomes() {
        let accounts = row_accounts(&request);
        let pairs = request["intra_pairs"].as_array().unwrap();
        let audit = response["intra_audit"].as_array().unwrap();
        let mut flow: BTreeMap<String, i128> = BTreeMap::new();
        let mut balance: BTreeMap<(String, String), i128> = BTreeMap::new();
        for entry in response["entries"].as_array().unwrap() {
            let kind = entry["entry_type"].as_str().unwrap();
            let (account, quantity) = match kind {
                "acquisition" | "disposal" | "fee" | "transfer_fee" => (
                    accounts[entry["row"].as_str().unwrap()].clone(),
                    msat(&entry["quantity"]),
                ),
                "transfer_out" | "transfer_in" => {
                    let row = &audit[entry["audit"].as_u64().unwrap() as usize];
                    let pair = &pairs[row["pair"].as_u64().unwrap() as usize];
                    if kind == "transfer_out" {
                        (
                            accounts[pair["out"].as_str().unwrap()].clone(),
                            -msat(&row["sent"]),
                        )
                    } else {
                        (
                            accounts[pair["in"].as_str().unwrap()].clone(),
                            msat(&row["received"]),
                        )
                    }
                }
                _ => continue,
            };
            if kind != "transfer_out" && kind != "transfer_in" {
                *flow.entry(account.0.clone()).or_default() += quantity;
            }
            // A move's fee is in what it sent.
            if kind != "transfer_fee" {
                *balance.entry(account).or_default() += quantity;
            }
        }
        let mut held: BTreeMap<String, i128> = BTreeMap::new();
        let mut held_by_wallet: BTreeMap<(String, String), i128> = BTreeMap::new();
        for holding in response["holdings"].as_array().unwrap() {
            let asset = holding["asset"].as_str().unwrap().to_owned();
            *held.entry(asset.clone()).or_default() += msat(&holding["quantity"]);
            held_by_wallet.insert(
                (asset, holding["wallet"].as_str().unwrap().to_owned()),
                msat(&holding["quantity"]),
            );
        }
        flow.retain(|_, value| *value != 0);
        assert_eq!(flow, held, "seed {seed}");
        for (account, value) in &balance {
            assert!(*value >= 0, "seed {seed}: {account:?} ends at {value} msat");
        }
        balance.retain(|_, value| *value != 0);
        assert_eq!(balance, held_by_wallet, "seed {seed}");
        for row in audit {
            let (sent, received, fee) = (
                row["sent_msat"].as_u64().unwrap(),
                row["received_msat"].as_u64().unwrap(),
                row["fee_msat"].as_u64().unwrap(),
            );
            assert_eq!(sent, received + fee, "seed {seed}");
            moves += 1;
        }
    }
    assert!(moves > SEEDS as usize, "only {moves} audited moves");
}

/// A description's `key=` value, if it has one.
fn marker<'t>(text: &'t str, key: &str) -> Option<&'t str> {
    text.split(' ')
        .find_map(|token| token.strip_prefix(key)?.strip_prefix('='))
}

#[test]
fn austrian_entries_carry_their_regime_and_moves_their_regime_flows() {
    let (mut alt, mut neu, mut mixed, mut overridden) = (0, 0, 0, 0);
    for (seed, request, response) in austrian_outcomes() {
        let rows: HashMap<&str, &Value> = request["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row["id"].as_str().unwrap(), row))
            .collect();
        for entry in response["entries"].as_array().unwrap() {
            if entry["entry_type"] == "transfer_out" || entry["entry_type"] == "transfer_in" {
                continue;
            }
            let text = entry["description"].as_str().unwrap();
            // The engine's regime and pool come first; the row's text after.
            let regime = text
                .split(' ')
                .next()
                .and_then(|token| token.strip_prefix("at_regime="));
            assert!(matches!(regime, Some("alt" | "neu")), "seed {seed}: {text}");
            assert!(
                marker(text, "at_pool") == Some("default"),
                "seed {seed}: {text}"
            );
            let row = rows[entry["row"].as_str().unwrap()];
            if let Some(chosen) = row["regime_override"].as_str() {
                assert_eq!(regime, Some(chosen), "seed {seed}: {text}");
                assert!(
                    marker(text, "at_regime_basis").is_none(),
                    "seed {seed}: {text}"
                );
                overridden += 1;
            }
        }
        for audit in response["intra_audit"].as_array().unwrap() {
            let flows = &audit["regime_flows"];
            let value = |side: &str, regime: &str| flows[side][regime].as_u64().unwrap();
            let (sent, received, fee) = (
                audit["sent_msat"].as_u64().unwrap(),
                audit["received_msat"].as_u64().unwrap(),
                audit["fee_msat"].as_u64().unwrap(),
            );
            let (out_total, in_total) = (
                value("out", "alt") + value("out", "neu"),
                value("in", "alt") + value("in", "neu"),
            );
            // What reached the destination left the source, per regime.
            // The fee is the first slice of what left, so the difference is
            // the fee, or all that left when that was less; nothing beyond
            // what the move sent left, and when all of it left, all it
            // received arrived.
            assert!(
                value("in", "alt") <= value("out", "alt"),
                "seed {seed}: {audit}"
            );
            assert!(
                value("in", "neu") <= value("out", "neu"),
                "seed {seed}: {audit}"
            );
            assert_eq!(
                out_total - in_total,
                fee.min(out_total),
                "seed {seed}: {audit}"
            );
            assert!(out_total <= sent, "seed {seed}: {audit}");
            if out_total == sent {
                assert_eq!(in_total, received, "seed {seed}: {audit}");
            }
            assert!(
                matches!(audit["at_regime"].as_str(), Some("alt" | "neu")),
                "seed {seed}: {audit}"
            );
            let text = audit["description"].as_str().unwrap();
            assert_eq!(
                marker(text, "at_regime"),
                audit["at_regime"].as_str(),
                "seed {seed}: {text}"
            );
            assert_eq!(
                marker(text, "at_alt_out"),
                Some(value("out", "alt").to_string().as_str()),
                "seed {seed}"
            );
            assert_eq!(
                marker(text, "at_alt_in"),
                Some(value("in", "alt").to_string().as_str()),
                "seed {seed}"
            );
            match (value("out", "alt") > 0, value("out", "neu") > 0) {
                (true, true) => mixed += 1,
                (true, false) => alt += 1,
                (false, true) => neu += 1,
                (false, false) => {}
            }
        }
    }
    // The books reach Alt, Neu, and mixed moves, and overrides.
    assert!(
        alt > 0 && neu > 0 && mixed > 0,
        "{alt} Alt, {neu} Neu, {mixed} mixed moves"
    );
    assert!(
        overridden > SEEDS as usize / 10,
        "only {overridden} overridden entries"
    );
}

#[test]
fn austrian_categories_follow_the_reviewed_overrides() {
    let mut cleared = 0;
    let mut replaced = 0;
    for (seed, request, response) in austrian_outcomes() {
        let rows: HashMap<&str, &Value> = request["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row["id"].as_str().unwrap(), row))
            .collect();
        for entry in response["entries"].as_array().unwrap() {
            let kind = entry["entry_type"].as_str().unwrap();
            if !matches!(kind, "income" | "disposal" | "fee" | "transfer_fee") {
                continue;
            }
            let category = entry["at_category"].as_str();
            if matches!(kind, "fee" | "transfer_fee") {
                assert_eq!(category, None, "seed {seed}: a fee reports no category");
                continue;
            }
            let row = rows[entry["row"].as_str().unwrap()];
            let chosen = row["category_override"].as_str();
            let off = row["taxability_off"] == true;
            match (off, chosen) {
                (true, Some(chosen @ ("alt_taxfree" | "neu_swap"))) => {
                    assert_eq!(category, Some(chosen), "seed {seed}");
                    replaced += 1;
                }
                (true, _) | (false, Some("none")) => {
                    assert_eq!(category, None, "seed {seed}");
                    cleared += 1;
                }
                (false, Some(chosen)) => {
                    assert_eq!(category, Some(chosen), "seed {seed}");
                    replaced += 1;
                }
                (false, None) => assert!(category.is_some(), "seed {seed}: {entry}"),
            }
        }
    }
    assert!(
        cleared > 0 && replaced > 0,
        "{cleared} cleared, {replaced} replaced"
    );
}

/// Property 4 on Austrian books, with the same exclusion and one more.
/// Pricing can release a disposal or a move fee the Austrian rules tag Neu
/// where no Neu lot covers it, which then aborts the computation: the Neu
/// shortfall (TODO "Stop an Austrian Neu shortfall from aborting the whole
/// report"). The oracle does the same; every such book up to 3,000 seeds
/// was replayed on it (WI-9 in the implementation log), and the goldens
/// `at-pricing-releases-a-neu-shortfall-*` pin the smallest one. Such an
/// abort is excused and counted, and nothing else is.
#[test]
fn pricing_an_unpriced_austrian_inbound_row_quarantines_no_new_row() {
    let mut repriced = 0;
    let mut excused = 0;
    let mut released_shortfalls = 0;
    for (seed, request, before) in austrian_outcomes() {
        let rows = request["rows"].as_array().unwrap();
        for index in 0..rows.len() {
            let row = &rows[index];
            if row["direction"] != "inbound" || !row["rate"].is_null() || !row["value"].is_null() {
                continue;
            }
            let mut priced = request.clone();
            priced["rows"][index]["rate"] = json!("4000000E-2");
            priced["rows"][index]["coarse"] = json!(false);
            let after = run(&priced);
            repriced += 1;
            if after["ok"] != true {
                assert!(
                    is_neu_shortfall(&after),
                    "seed {seed}, row {index}: {after}"
                );
                released_shortfalls += 1;
                continue;
            }
            let (added, released) = unexcused_additions(&request, &before, &after);
            assert!(
                added.is_empty(),
                "seed {seed}: pricing row {index} quarantines {added:?}"
            );
            excused += released;
        }
    }
    assert!(
        repriced > SEEDS as usize,
        "only {repriced} inbound rows repriced"
    );
    // Both exclusions are used, and rarely.
    assert!(
        (1..=SEEDS as usize / 20).contains(&excused),
        "{excused} rows excused"
    );
    assert!(
        (1..=SEEDS as usize / 50).contains(&released_shortfalls),
        "{released_shortfalls} released Neu shortfalls"
    );
}

#[test]
fn pricing_an_austrian_move_can_release_a_neu_shortfall() {
    // The smallest book of the Austrian exclusion, as the goldens
    // `at-pricing-releases-a-neu-shortfall-*` store the oracle's outcome:
    // a move out of an Alt-only wallet whose regime the user set to Neu.
    let row =
        |id: &str, wallet: &str, direction: &str, at: i64, amount: u64, fee: u64, rate: Value| {
            json!({
                "id": format!("custody-tax:{id}"),
                "anchor": id.trim_end_matches("-out").trim_end_matches("-in"),
                "asset": "BTC",
                "wallet_id": wallet,
                "direction": direction,
                "ts": ts(at),
                "cutoff_instant": {"us": at * 1_000_000},
                "amount_msat": amount,
                "fee_msat": fee,
                "rate": rate,
                "value": null,
                "coarse": false,
                "kind": if direction == "inbound" { "buy" } else { "transfer" },
                "text": id,
                "regime_override": if id == "move-out" { json!("neu") } else { Value::Null },
                "category_override": null,
                "taxability_off": false,
                "privacy_hop": false,
                "hold": null,
                "chain_order": null,
            })
        };
    let request = |in_rate: Value| {
        let mut request = austrian_book(0);
        request["policy"]["method"] = json!("moving_average_at");
        request["policy"]["require_coarse_review"] = json!(false);
        request["rows"] = json!([
            row(
                "alt-buy",
                "C",
                "inbound",
                1_609_459_200,
                100_000_000_000,
                0,
                json!("30000E0")
            ),
            row(
                "move-out",
                "C",
                "outbound",
                1_612_137_600,
                50_000_000_000,
                100_000_000,
                Value::Null
            ),
            row(
                "move-in",
                "B",
                "inbound",
                1_612_138_200,
                50_000_000_000,
                0,
                in_rate
            ),
        ]);
        request["intra_pairs"] = json!([{
            "out": "custody-tax:move-out",
            "in": "custody-tax:move-in",
            "source": "reviewed_custody_component",
            "group_id": "bundle:move",
        }]);
        request
    };
    let before = run(&request(Value::Null));
    assert_eq!(
        quarantined_rows(&before).into_iter().collect::<Vec<_>>(),
        vec![
            (
                "custody-tax:move-in".to_owned(),
                "missing_spot_price".to_owned()
            ),
            (
                "custody-tax:move-out".to_owned(),
                "missing_spot_price".to_owned()
            ),
        ]
    );
    let after = run(&request(json!("40000E0")));
    assert!(is_neu_shortfall(&after), "{after}");
}
