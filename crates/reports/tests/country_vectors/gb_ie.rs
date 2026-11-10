//! Ireland's and the United Kingdom's vectors through their registry
//! entries: each vector's events become a `ledger` request under the
//! entry's engine rules and default method, the engine computes it, and the
//! country layer (`kassiber_reports::ie_tax`, `kassiber_reports::gb_tax`)
//! turns the response into the vector's `expected` shape.

use super::*;
use kassiber_tax_countries::Country;

/// What the request left outside the engine's values: per event, fiat
/// costs of the disposal the vector states apart (a sale's commission, half
/// a swap's fee), and each row's kind.
#[derive(Default)]
pub(super) struct Outside {
    pub costs: BTreeMap<String, Decimal>,
    pub kinds: BTreeMap<String, String>,
}

/// The ledger request of a vector under `entry`: rows as the custody
/// projection writes them, the entry's rules and its default method.
pub(super) fn request(vector: &Value, entry: &Country) -> (Value, Outside) {
    let (mut rows, mut moves, mut cross) = (Vec::new(), Vec::new(), Vec::new());
    let (mut assets, mut wallets) = (BTreeSet::new(), BTreeSet::new());
    let mut outside = Outside::default();
    let canonical_of = |d: Decimal| json!(d.to_canonical());
    let two = Decimal::from(2u8);
    for event in vector["events"].as_array().unwrap() {
        let id = event["id"].as_str().unwrap();
        let time = event["time"].as_str().unwrap();
        let wallet = event["wallet"].as_str().unwrap();
        let quantity = event["quantity"].as_str().unwrap();
        let asset = &event["asset"];
        assets.insert(asset.as_str().unwrap().to_owned());
        wallets.insert(wallet.to_owned());
        let fiat = field(event, "fiat");
        let fee_fiat = field(event, "fee_fiat").unwrap_or_else(Decimal::zero);
        let mut push = |rows: &mut Vec<Value>, row: Value| {
            outside
                .kinds
                .insert(id.to_owned(), row["kind"].as_str().unwrap().to_owned());
            rows.push(row);
        };
        let row = |direction: &str, kind: &str| {
            probe_row(id, asset, wallet, direction, time, quantity, kind)
        };
        match event["type"].as_str().unwrap() {
            "buy" | "wages" => {
                let mut r = row("inbound", "buy");
                r["value"] = canonical_of(fiat.unwrap().add(&fee_fiat, &CTX).unwrap());
                push(&mut rows, r);
            }
            kind @ ("sell" | "spend" | "gift_out" | "lost") => {
                let mut r = row(
                    "outbound",
                    match kind {
                        "gift_out" => "gift",
                        "lost" => "lost",
                        "spend" => "spend",
                        _ => "sell",
                    },
                );
                if let Some(fiat) = fiat {
                    // A book nets a fiat commission into the value; the
                    // layer adds it back as a disposal cost.
                    r["value"] = canonical_of(fiat.sub(&fee_fiat, &CTX).unwrap());
                    if !fee_fiat.is_zero() {
                        outside.costs.insert(id.to_owned(), fee_fiat.clone());
                    }
                }
                // The fee units take the sale's price, which is the value
                // the vectors give them.
                if let Some(fee) = event["fee_quantity"].as_str() {
                    r["fee_msat"] = json!(msat_of(fee));
                }
                push(&mut rows, r);
            }
            "income" | "airdrop" => {
                let subkind = event["subkind"].as_str().unwrap_or("");
                let kind = match PROBE_INCOME.iter().find(|(name, _)| *name == subkind) {
                    Some((_, kind)) if subkind != "airdrop_service" => *kind,
                    // Kassiber has no kind for a reviewed service airdrop.
                    _ if event["type"] == "airdrop" => "airdrop",
                    _ => subkind,
                };
                let mut r = row("inbound", kind);
                r["value"] = canonical_of(fiat.unwrap());
                push(&mut rows, r);
            }
            "transfer" => {
                let to = event["to_wallet"].as_str().unwrap();
                wallets.insert(to.to_owned());
                let mut out = probe_row(
                    &format!("{id}-out"),
                    asset,
                    wallet,
                    "outbound",
                    time,
                    quantity,
                    "transfer",
                );
                let mut inbound = probe_row(
                    &format!("{id}-in"),
                    asset,
                    to,
                    "inbound",
                    time,
                    quantity,
                    "transfer",
                );
                if let (Some(fee), Some(value)) = (
                    event["fee_quantity"].as_str(),
                    field(event, "fee_fiat_value"),
                ) {
                    out["fee_msat"] = json!(msat_of(fee));
                    let rate = canonical_of(value.div(&dec(fee), &CTX).unwrap());
                    out["rate"] = rate.clone();
                    inbound["rate"] = rate;
                }
                outside.kinds.insert(id.to_owned(), "transfer".into());
                rows.extend([out, inbound]);
                moves.push(json!({"out": format!("custody-tax:{id}-out"), "in": format!("custody-tax:{id}-in"),
                                  "source": "reviewed_custody_component", "group_id": null}));
            }
            "swap" => {
                let in_asset = &event["in_asset"];
                assets.insert(in_asset.as_str().unwrap().to_owned());
                // A fiat fee belongs half to each asset (GB-FEE-SWAP): the
                // received value nets the disposal's half, which the layer
                // adds back as a disposal cost, and the given value carries
                // the acquisition's half into the received asset's cost.
                let half = fee_fiat.div(&two, &CTX).unwrap();
                let mut out = probe_row(
                    &format!("{id}-out"),
                    asset,
                    wallet,
                    "outbound",
                    time,
                    quantity,
                    "swap",
                );
                out["value"] = canonical_of(fiat.unwrap().add(&half, &CTX).unwrap());
                let mut inbound = probe_row(
                    &format!("{id}-in"),
                    in_asset,
                    wallet,
                    "inbound",
                    time,
                    event["in_quantity"].as_str().unwrap(),
                    "swap",
                );
                if let Some(value) = field(event, "in_fiat") {
                    inbound["value"] = canonical_of(value.sub(&half, &CTX).unwrap());
                }
                if !half.is_zero() {
                    outside.costs.insert(id.to_owned(), half);
                }
                outside.kinds.insert(id.to_owned(), "swap".into());
                rows.extend([out, inbound]);
                cross.push(
                    json!({"pair_id": id, "out_id": format!("custody-tax:{id}-out"),
                                  "in_id": format!("custody-tax:{id}-in"), "out_transaction_id": id,
                                  "in_transaction_id": id, "out_asset": asset, "in_asset": in_asset,
                                  "policy": "taxable", "bitcoin_rail": false}),
                );
            }
            other => panic!("{}: {other} events are not modelled", vector["id"]),
        }
    }
    let method = entry.default_method.to_lowercase();
    assert_eq!(
        vector["policy"]["method"].as_str(),
        Some(method.as_str()),
        "{}: the vector's method is the entry's",
        vector["id"]
    );
    let policy = json!({"country": entry.engine_country().unwrap().unwrap(), "method": method, "method_label": entry.default_method,
                        "require_coarse_review": false});
    let request = json!({
        "schema_version": 2, "operation": "ledger", "policy": policy,
        "config": {"assets": assets, "exchanges": wallets, "holders": ["H"]},
        "holder": "H",
        "wallets": wallets.iter().map(|w| json!({"id": w, "label": w})).collect::<Vec<_>>(),
        "rows": rows, "intra_pairs": moves, "cross_asset_pairs": cross,
    });
    (request, outside)
}

/// A response row's vector event (`custody-tax:x1-out` is `x1`).
pub(super) fn event_of_row(row: &str) -> String {
    let row = row.trim_start_matches("custody-tax:");
    row.strip_suffix("-out")
        .or(row.strip_suffix("-in"))
        .unwrap_or(row)
        .to_owned()
}

pub(super) fn instant_of(vector: &Value, event: &str) -> i64 {
    let event = vector["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == event)
        .unwrap_or_else(|| panic!("no event {event}"));
    parse_instant(event["time"].as_str().unwrap()).unwrap().us
}

fn event_type<'a>(vector: &'a Value, event: &str) -> &'a str {
    vector["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == event)
        .and_then(|e| e["type"].as_str())
        .unwrap_or("")
}

pub(super) fn run_ledger(request: &Value) -> Value {
    serde_json::from_str(&kassiber_tax_core::ledger(&request.to_string())).unwrap()
}

/// Ireland: every vector, computed in full.
pub(super) fn ie_compute(vector: &Value) -> Value {
    use kassiber_reports::ie_tax::{
        self, Disposal, Income, Kind, Lot, Purchase, Quarantine, Settings,
    };
    let entry = kassiber_tax_countries::lookup("ie").unwrap();
    let (request, outside) = request(vector, entry);
    let response = run_ledger(&request);
    assert_eq!(response["ok"], true, "{}: {response}", vector["id"]);
    let (mut disposals, mut income) = (Vec::new(), Vec::new());
    for item in response["entries"].as_array().unwrap() {
        let entry_type = item["entry_type"].as_str().unwrap();
        if !matches!(entry_type, "disposal" | "fee" | "transfer_fee" | "income") {
            continue;
        }
        let event = event_of_row(item["row"].as_str().unwrap());
        let us = instant_of(vector, &event);
        let quantity = dec_canonical(&item["quantity"]).copy_abs();
        if entry_type == "income" {
            let source = vector["events"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == event.as_str())
                .unwrap();
            income.push(Income {
                transaction_id: event,
                wallet: item["wallet"].as_str().unwrap().into(),
                asset: item["asset"].as_str().unwrap().into(),
                kind: source["subkind"].as_str().unwrap().into(),
                us,
                quantity,
                value: dec_canonical(&item["proceeds"]),
            });
            continue;
        }
        let kind = match (entry_type, event_type(vector, &event)) {
            ("fee" | "transfer_fee", _) | (_, "transfer") => Kind::NetworkFee,
            (_, "swap") => Kind::Swap,
            (_, "spend") => Kind::Spend,
            (_, "gift_out") => Kind::Gift,
            _ => Kind::Sale,
        };
        let lots = item["fragments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|fragment| Lot {
                transaction_id: event_of_row(fragment["lot"]["row"].as_str().unwrap()),
                quantity: dec_canonical(&fragment["crypto_amount"]),
                cost: dec_canonical(&fragment["cost_basis"]),
            })
            .collect();
        disposals.push(Disposal {
            transaction_id: event.clone(),
            wallet: item["wallet"].as_str().unwrap().into(),
            asset: item["asset"].as_str().unwrap().into(),
            kind,
            us,
            quantity,
            proceeds: dec_canonical(&item["proceeds"]),
            cost: dec_canonical(&item["cost_basis"]),
            disposal_costs: outside
                .costs
                .get(&event)
                .cloned()
                .unwrap_or_else(Decimal::zero),
            lots,
        });
    }
    let purchases: Vec<Purchase> = vector["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "buy")
        .map(|e| Purchase {
            asset: e["asset"].as_str().unwrap().into(),
            us: parse_instant(e["time"].as_str().unwrap()).unwrap().us,
        })
        .collect();
    let quarantines: Vec<Quarantine> = response["quarantines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            let event = event_of_row(q["row"].as_str().unwrap());
            Quarantine {
                code: ie_tax::quarantine_code(
                    q["reason"].as_str().unwrap(),
                    outside.kinds.get(&event).map_or("", String::as_str),
                    q["detail"]["code"].as_str(),
                ),
                transaction_id: event,
            }
        })
        .collect();
    let settings = Settings {
        losses_brought_forward_opening: field(&vector["policy"], "losses_brought_forward_opening")
            .unwrap_or_else(Decimal::zero),
        exemption_available: field(&vector["policy"], "ie_exemption_available"),
    };
    let (rows, flags) = ie_tax::place(&disposals, &purchases).unwrap();
    let years = ie_tax::years(&rows, &income, &settings).unwrap();
    ie_tax::vector_json(&rows, &flags, &income, &quarantines, &years).unwrap()
}

/// The United Kingdom: every vector, computed in full; a vector the rules
/// refuse names the refusal of the first transaction the engine held.
pub(super) fn gb_compute(vector: &Value) -> Value {
    use kassiber_reports::gb_tax::{self, Acquisition, Disposal, Income, Piece};
    let entry = kassiber_tax_countries::lookup("gb").unwrap();
    let (request, outside) = request(vector, entry);
    let response = run_ledger(&request);
    if response["ok"] != true {
        let message = response["error"]["message"].as_str().unwrap_or("");
        let code = if message.starts_with(LOTS_EXHAUSTED) {
            "insufficient_holdings"
        } else {
            "engine_error"
        };
        return json!({"refusal": {"code": code, "event": "-", "reason": message}});
    }
    if let Some(held) = response["quarantines"].as_array().unwrap().first() {
        let event = event_of_row(held["row"].as_str().unwrap());
        let code = gb_tax::quarantine_code(
            held["reason"].as_str().unwrap(),
            outside.kinds.get(&event).map_or("", String::as_str),
            held["detail"]["code"].as_str(),
        );
        return json!({"refusal": {"code": code, "event": event}});
    }
    let date_of = |event: &str| gb_tax::london_date(instant_of(vector, event)).unwrap();
    let (mut disposals, mut acquisitions, mut income) = (Vec::new(), Vec::new(), Vec::new());
    for item in response["entries"].as_array().unwrap() {
        let entry_type = item["entry_type"].as_str().unwrap_or("");
        if !matches!(
            entry_type,
            "disposal" | "fee" | "transfer_fee" | "income" | "acquisition"
        ) {
            continue;
        }
        let event = event_of_row(item["row"].as_str().unwrap());
        let date = date_of(&event);
        let asset: String = item["asset"].as_str().unwrap().into();
        let quantity = dec_canonical(&item["quantity"]).copy_abs();
        match entry_type {
            "acquisition" => acquisitions.push(Acquisition {
                asset,
                date,
                quantity,
                cost: dec_canonical(&item["fiat_value"]),
            }),
            "income" => {
                let source = vector["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|e| e["id"] == event.as_str())
                    .unwrap();
                income.push(Income {
                    transaction_id: event,
                    wallet: item["wallet"].as_str().unwrap().into(),
                    asset,
                    kind: source["subkind"].as_str().unwrap().into(),
                    date,
                    quantity,
                    value: dec_canonical(&item["proceeds"]),
                });
            }
            _ => {
                let pieces = item["fragments"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|fragment| Piece {
                        acquired: date_of(&event_of_row(fragment["lot"]["row"].as_str().unwrap())),
                        quantity: dec_canonical(&fragment["crypto_amount"]),
                        cost: dec_canonical(&fragment["cost_basis"]),
                    })
                    .collect();
                let proceeds = dec_canonical(&item["proceeds"]);
                let cost = dec_canonical(&item["cost_basis"]);
                // As the journal reader derives them: the entry's expenses
                // are not stored.
                let expenses =
                    gb_tax::expenses_of(&proceeds, &cost, &dec_canonical(&item["gain_loss"]))
                        .unwrap();
                disposals.push(Disposal {
                    event: if entry_type == "transfer_fee" {
                        format!("{event}:fee")
                    } else {
                        event.clone()
                    },
                    wallet: item["wallet"].as_str().unwrap().into(),
                    asset,
                    date,
                    quantity,
                    proceeds,
                    cost,
                    expenses,
                    incidental: outside
                        .costs
                        .get(&event)
                        .cloned()
                        .unwrap_or_else(Decimal::zero),
                    pieces,
                });
            }
        }
    }
    let (computations, history) = gb_tax::computations(&disposals, &acquisitions).unwrap();
    let years = gb_tax::years(&computations, &income, &history).unwrap();
    gb_tax::vector_json(
        &computations,
        &income,
        &years,
        &gb_tax::final_pools(&history),
    )
    .unwrap()
}

/// RP2's IE plugin on the same inputs (`rp2.computed` in each vector, see
/// `docs/reference/tax-country-ie.md#rp2-comparison`): every lot it takes
/// is Kassiber's lot, at the same quantity and cost, and every gain is
/// Kassiber's but for the differences the specification records.
#[test]
fn ireland_agrees_with_rp2_but_for_the_recorded_differences() {
    let text =
        std::fs::read_to_string(reference_dir().join("tax-country-ie-vectors.json")).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    let mut differences = BTreeSet::new();
    for vector in doc["vectors"].as_array().unwrap() {
        let id = vector["id"].as_str().unwrap();
        let ours = ie_compute(vector);
        let rp2 = &vector["rp2"]["computed"];
        assert_eq!(rp2["status"], "ok", "{id}");
        for gain in rp2["gains"].as_array().unwrap() {
            let event = gain["event"].as_str().unwrap();
            let kind = gain["type"].as_str().unwrap();
            let Some(disposal) = ours["disposals"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["event"] == event)
            else {
                // RP2 taxes interest as a gain and books lost coins as a
                // loss; Kassiber books the interest as Case IV income and
                // holds the lost coins (IE-R9, IE-R13).
                let income = ours["income"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|i| i["event"] == event);
                let held = ours["quarantines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|q| q["event"] == event);
                assert!(
                    (kind == "interest" && income) || (kind == "lost" && held),
                    "{id}: RP2's {event} ({kind}) has no Kassiber disposal"
                );
                differences.insert(format!("{id}:{kind}"));
                continue;
            };
            let lot = disposal["lots"]
                .as_array()
                .unwrap()
                .iter()
                .find(|l| l["lot"] == gain["lot"])
                .unwrap_or_else(|| panic!("{id}: RP2 took lot {} for {event}", gain["lot"]));
            assert_eq!(
                (&lot["qty"], &lot["cost"]),
                (&gain["qty"], &gain["cost_basis"]),
                "{id} {event}"
            );
            if disposal["lots"].as_array().unwrap().len() == 1 && disposal["gain"] != gain["gain"] {
                // A sale's disposal costs (IE-R2), and the received coins'
                // value as a swap's consideration (IE-Q2).
                let source = vector["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|e| e["id"] == event)
                    .unwrap();
                let delta = dec(disposal["gain"].as_str().unwrap())
                    .sub(&dec(gain["gain"].as_str().unwrap()), &CTX)
                    .unwrap();
                let expected = match source["type"].as_str().unwrap() {
                    "sell" => field(source, "fee_fiat").unwrap().neg(&CTX).unwrap(),
                    "swap" => field(source, "in_fiat")
                        .unwrap()
                        .sub(&field(source, "fiat").unwrap(), &CTX)
                        .unwrap(),
                    other => panic!("{id}: an unrecorded difference on a {other}"),
                };
                assert_eq!(delta, expected, "{id} {event}");
                differences.insert(format!("{id}:{}", source["type"].as_str().unwrap()));
            }
        }
    }
    // Exactly the differences the specification records.
    assert_eq!(
        differences.into_iter().collect::<Vec<_>>(),
        ["ie-05:sell", "ie-06:swap", "ie-14:interest", "ie-17:lost"]
    );
}
