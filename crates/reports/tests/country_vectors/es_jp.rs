//! Spain's and Japan's vectors through their registry entries: each
//! vector's events become a `ledger` request under the entry's engine rules
//! (`probe_request`), the engine computes it, and the country layer
//! (`kassiber_reports::es_tax`, `kassiber_reports::jp_tax`) turns the
//! response into the vector's `expected` shape.

use super::*;

/// The ledger response of a vector under its registry entry's rules, as a
/// book of the country sends them; `method` replaces the vector's when the
/// country names its methods otherwise.
fn registry_run(code: &str, vector: &Value, method: Option<&str>) -> Value {
    let mut request = probe_request(vector, registry_rules(code))
        .unwrap_or_else(|reason| panic!("{}: {reason}", vector["id"]));
    if let Some(method) = method {
        request["policy"]["method"] = json!(method);
        request["policy"]["method_label"] = json!(method.to_uppercase());
    }
    serde_json::from_str(&kassiber_tax_core::ledger(&request.to_string())).unwrap()
}

/// A refused row's country code, else the engine's reason.
fn reason(quarantine: &Value) -> Value {
    match &quarantine["detail"]["code"] {
        Value::String(code) => json!(code),
        _ => quarantine["reason"].clone(),
    }
}

/// The vector event a ledger row names (`custody-tax:t2-out` is `t2`).
fn event_id(row: &Value) -> String {
    let row = row.as_str().unwrap().trim_start_matches("custody-tax:");
    row.strip_suffix("-out")
        .or(row.strip_suffix("-in"))
        .unwrap_or(row)
        .to_owned()
}

/// The vector events, and the position and instant of one by id.
struct Events<'v>(&'v [Value]);

impl<'v> Events<'v> {
    fn of(vector: &'v Value) -> Self {
        Events(vector["events"].as_array().unwrap())
    }
    fn position(&self, id: &str) -> usize {
        self.0
            .iter()
            .position(|event| event["id"] == id)
            .unwrap_or_else(|| panic!("no event {id}"))
    }
    fn get(&self, id: &str) -> &'v Value {
        &self.0[self.position(id)]
    }
    fn us(&self, id: &str) -> i64 {
        parse_instant(self.get(id)["time"].as_str().unwrap())
            .unwrap()
            .us
    }
}

/// The quarantines of a response, by vector event: a refused kind's
/// country code, else the engine's reason.
fn registry_quarantines(response: &Value) -> Vec<Value> {
    response["quarantines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| json!({"event": event_id(&q["row"]), "reason": reason(q)}))
        .collect()
}

/// Spain: the vector through the engine under Spain's registry rules, then
/// the Spanish layer (`kassiber_reports::es_tax`) over the journal entries
/// the engine returned.
pub(super) fn es_compute(vector: &Value) -> Value {
    use kassiber_reports::es_tax::{self, Acquisition, Disposal, Income, Kind, Lot};
    let response = registry_run("es", vector, None);
    if response["ok"] != true {
        return json!({"refusal": {"code": "engine_error", "event": "-"}, "error": response});
    }
    let zone = calendar::zone("Europe/Madrid").unwrap();
    let events = Events::of(vector);
    let (mut disposals, mut income, mut acquisitions) = (Vec::new(), Vec::new(), Vec::new());
    for entry in response["entries"].as_array().unwrap() {
        let Some(row) = entry.get("row") else {
            continue;
        };
        let id = event_id(row);
        let event = events.get(&id);
        let us = events.us(&id);
        let date = zone.local_date(us).unwrap();
        let asset = entry["asset"].as_str().unwrap().to_owned();
        let quantity = dec_canonical(&entry["quantity"]).copy_abs();
        match entry["entry_type"].as_str().unwrap() {
            "acquisition" => acquisitions.push(Acquisition {
                transaction_id: id,
                asset,
                us,
                date,
            }),
            "income" => income.push(Income {
                transaction_id: id,
                wallet: entry["wallet"].as_str().unwrap().to_owned(),
                asset,
                us,
                date,
                kind: event["subkind"].as_str().unwrap().to_owned(),
                quantity,
                value: dec_canonical(&entry["proceeds"]),
            }),
            "disposal" | "fee" | "transfer_fee" => disposals.push(Disposal {
                kind: match event["type"].as_str().unwrap() {
                    "sell" => Kind::Sale,
                    "swap" => Kind::Swap,
                    "spend" => Kind::Spend,
                    "gift_out" => Kind::Gift,
                    _ => Kind::NetworkFee,
                },
                transaction_id: id,
                wallet: entry["wallet"].as_str().unwrap().to_owned(),
                asset,
                us,
                date,
                quantity,
                proceeds: dec_canonical(&entry["proceeds"]),
                cost: dec_canonical(&entry["cost_basis"]),
                expenses: entry
                    .get("expenses")
                    .map_or_else(Decimal::zero, dec_canonical),
                computable: entry.get("loss_deductible") != Some(&json!(false)),
                valuation: entry
                    .get("valuation")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                lots: entry["fragments"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|fragment| Lot {
                        transaction_id: event_id(&fragment["lot"]["row"]),
                        quantity: dec_canonical(&fragment["crypto_amount"]),
                        cost: dec_canonical(&fragment["cost_basis"]),
                    })
                    .collect(),
            }),
            _ => {}
        }
    }
    disposals.sort_by_key(|d| events.position(&d.transaction_id));
    income.sort_by_key(|i| events.position(&i.transaction_id));
    let money = |d: &Decimal| es_tax::money(d).unwrap();
    let rows: Vec<Value> = disposals
        .iter()
        .map(|d| {
            let row = es_tax::row(d).unwrap();
            json!({
                "event": d.transaction_id, "asset": d.asset, "qty": qty(&d.quantity),
                "kind": d.kind.code(), "contraprestacion": d.kind.contraprestacion(),
                "valor_transmision": money(&row.valor_transmision),
                "valor_adquisicion": money(&row.valor_adquisicion),
                "ganancia": money(&row.ganancia), "computable": d.computable,
                "lots": d.lots.iter().map(|lot| json!({"lot": lot.transaction_id,
                    "qty": qty(&lot.quantity), "cost": money(&lot.cost)})).collect::<Vec<_>>(),
            })
        })
        .collect();
    let receipts: Vec<Value> = income
        .iter()
        .map(|i| {
            let (casilla, base) = es_tax::casilla(&i.kind).unwrap_or(("\u{2014}", "\u{2014}"));
            json!({"event": i.transaction_id, "asset": i.asset, "kind": i.kind,
                   "casilla": casilla, "base": base, "value": money(&i.value)})
        })
        .collect();
    let flags: Vec<Value> = es_tax::flags(&disposals, &income, &acquisitions)
        .unwrap()
        .into_iter()
        .map(|flag| json!({"event": flag.event, "code": flag.code}))
        .collect();
    let mut years = Map::new();
    for year in es_tax::years(&disposals, &income).unwrap() {
        years.insert(year.year.to_string(), es_tax::year_json(&year).unwrap());
    }
    json!({"disposals": rows, "income": receipts, "quarantines": registry_quarantines(&response),
           "flags": flags, "years": years})
}

/// Kassiber records a sale's yen commission netted into its price, as for
/// every country (`docs/reference/tax-country-jp.md`, JP-Q12): the
/// specification lists it as その他の必要経費 apart from a gross revenue.
/// The expected figures are restated that way: the sale's coin and Tokyo
/// year lose the commission from revenue and other expenses, and from both
/// rounded totals (whole yen, so their rounding is unchanged); the income
/// stays.
pub(super) fn net_jp_commissions(vector: &Value, expected: &mut Value) {
    for event in vector["events"].as_array().unwrap() {
        let Some(commission) = field(event, "fee_fiat") else {
            continue;
        };
        if !matches!(event["type"].as_str(), Some("sell" | "spend")) {
            continue;
        }
        assert!(
            !event["fee_fiat"].as_str().unwrap().contains('.'),
            "{}: a whole-yen commission",
            event["id"]
        );
        let year = local_date(vector, event["id"].as_str().unwrap(), "Asia/Tokyo").year;
        let block =
            &mut expected["years"][year.to_string()]["assets"][event["asset"].as_str().unwrap()];
        for key in [
            "revenue",
            "other_expenses",
            "revenue_rounded",
            "expenses_rounded",
        ] {
            let value = dec(block[key].as_str().unwrap())
                .sub(&commission, &CTX)
                .unwrap();
            block[key] = json!(value.to_string());
        }
    }
}

/// Japan: the vector through the engine under Japan's registry rules (its
/// averages on the `Matching` hook), then the Japanese layer
/// (`kassiber_reports::jp_tax`) over the journal entries the engine
/// returned.
pub(super) fn jp_compute(vector: &Value) -> Value {
    use kassiber_reports::jp_tax::{self, Acquisition, Disposal, Income, Method, Quarantine};
    let method = match vector["policy"]["method"].as_str().unwrap() {
        "moving_average" => Method::MovingAverage,
        _ => Method::TotalAverage,
    };
    let response = registry_run(
        "jp",
        vector,
        Some(match method {
            Method::MovingAverage => "jp_moving_average",
            Method::TotalAverage => "jp_total_average",
        }),
    );
    if response["ok"] != true {
        return json!({"refusal": {"code": "engine_error", "event": "-"}, "error": response});
    }
    let zone = calendar::zone("Asia/Tokyo").unwrap();
    let events = Events::of(vector);
    let (mut acquisitions, mut disposals, mut income) = (Vec::new(), Vec::new(), Vec::new());
    for entry in response["entries"].as_array().unwrap() {
        let Some(row) = entry.get("row") else {
            continue;
        };
        let id = event_id(row);
        let event = events.get(&id);
        let us = events.us(&id);
        let date = zone.local_date(us).unwrap();
        let asset = entry["asset"].as_str().unwrap().to_owned();
        let quantity = dec_canonical(&entry["quantity"]).copy_abs();
        let wallet = entry["wallet"].as_str().unwrap().to_owned();
        match entry["entry_type"].as_str().unwrap() {
            "acquisition" => acquisitions.push(Acquisition {
                transaction_id: id,
                asset,
                us,
                date,
                quantity,
                value: dec_canonical(&entry["fiat_value"]),
            }),
            "income" => income.push(Income {
                transaction_id: id,
                wallet,
                asset,
                us,
                date,
                kind: event["subkind"].as_str().unwrap().to_owned(),
                quantity,
                value: dec_canonical(&entry["proceeds"]),
            }),
            kind @ ("disposal" | "fee" | "transfer_fee") => disposals.push(Disposal {
                transaction_id: id,
                wallet,
                asset,
                us,
                date,
                quantity,
                revenue: dec_canonical(&entry["proceeds"]),
                cost: dec_canonical(&entry["cost_basis"]),
                expenses: entry
                    .get("expenses")
                    .map_or_else(Decimal::zero, dec_canonical),
                network_fee: kind == "transfer_fee",
                valuation: entry
                    .get("valuation")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }),
            _ => {}
        }
    }
    let quarantines: Vec<Quarantine> = response["quarantines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            let id = event_id(&q["row"]);
            let event = events.get(&id);
            Quarantine {
                asset: event["asset"].as_str().unwrap().to_owned(),
                date: zone.local_date(events.us(&id)).unwrap(),
                reason: reason(q).as_str().unwrap().to_owned(),
                transaction_id: id,
            }
        })
        .collect();
    let (years, refusals) =
        jp_tax::years(method, &acquisitions, &disposals, &income, &quarantines).unwrap();
    let mut out_years = Map::new();
    for year in &years {
        out_years.insert(year.year.to_string(), jp_tax::year_json(year, method));
    }
    let flags: Vec<Value> = jp_tax::flags(&years, &disposals)
        .into_iter()
        .map(|flag| json!({"event": flag.event, "code": flag.code}))
        .collect();
    // A row the journal could not book: by its transaction, or by its coin
    // and year when the year's figures are withheld for it.
    let quarantined: Vec<Value> = quarantines
        .iter()
        .map(|q| {
            if jp_tax::withholds(&q.reason) {
                json!({"event": format!("{}:{}", q.date.year, q.asset), "reason": "missing_cost_basis"})
            } else {
                json!({"event": q.transaction_id, "reason": q.reason})
            }
        })
        .collect();
    json!({
        "quarantines": quarantined,
        "flags": flags,
        "refusals": refusals.iter().map(|r| json!({"year": r.year, "reason": r.reason})).collect::<Vec<_>>(),
        "years": out_years,
    })
}
