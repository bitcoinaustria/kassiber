//! The one runner for every country's vectors
//! (`docs/reference/tax-country-<code>-vectors.json`, schema
//! `kassiber.tax-country-vectors/1`, `docs/reference/tax-countries.md`).
//!
//! Every file is checked for the shared shape: the document's country,
//! currency, time zone and status; each vector's id, rules, policy (method
//! and pool scope) and events (id, ISO 8601 time with an offset, type,
//! wallet). A country the registry lists is then computed: its events become
//! tax-engine entries, the engine runs under the country and the vector's
//! pool scope, and the country layer totals the result; the vector's
//! `expected` must equal it field for field. A country the registry does not
//! list yet is research only and its vectors are not computed; once its
//! entry exists they are, and a country without a computation here fails.

use std::collections::{BTreeMap, BTreeSet};

#[path = "country_vectors/es_jp.rs"]
mod es_jp;
#[path = "country_vectors/gb_ie.rs"]
mod gb_ie;
use std::path::{Path, PathBuf};

use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_reports::us_tax::{self, Fee, Income, Row, Term};
use serde_json::{json, Map, Value};

const CTX: Context = Context::new(32, Rounding::HalfEven);
const LOTS_EXHAUSTED: &str = "Total in-transaction crypto value < total taxable crypto value";

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference")
}

fn files() -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(reference_dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(code) = name
            .strip_prefix("tax-country-")
            .and_then(|rest| rest.strip_suffix("-vectors.json"))
        {
            let text = std::fs::read_to_string(&path).unwrap();
            out.push((code.to_owned(), serde_json::from_str(&text).unwrap()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ------------------------------------------------------------- timestamps

/// A parsed `YYYY-MM-DDTHH:MM:SS[.ffffff](Z|±HH:MM)`.
struct Instant {
    raw: String,
    us: i64,
    offset_s: i32,
    /// The date in the timestamp's own offset.
    local: (i64, u32, u32),
    local_time: String,
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn parse_instant(raw: &str) -> Result<Instant, String> {
    let bad = || format!("{raw:?} is not an ISO 8601 time with an offset");
    let (main, offset_s) = if let Some(main) = raw.strip_suffix('Z') {
        (main, 0)
    } else {
        let at = raw.len().checked_sub(6).ok_or_else(bad)?;
        let (main, offset) = raw.split_at(at);
        let sign = match &offset[..1] {
            "+" => 1,
            "-" => -1,
            _ => return Err(bad()),
        };
        let hours: i32 = offset[1..3].parse().map_err(|_| bad())?;
        let minutes: i32 = offset[4..6].parse().map_err(|_| bad())?;
        (main, sign * (hours * 3600 + minutes * 60))
    };
    let (date, time) = main.split_once('T').ok_or_else(bad)?;
    let mut date_parts = date.split('-');
    let y: i64 = date_parts
        .next()
        .ok_or_else(bad)?
        .parse()
        .map_err(|_| bad())?;
    let m: u32 = date_parts
        .next()
        .ok_or_else(bad)?
        .parse()
        .map_err(|_| bad())?;
    let d: u32 = date_parts
        .next()
        .ok_or_else(bad)?
        .parse()
        .map_err(|_| bad())?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut clock_parts = clock.split(':');
    let hh: i64 = clock_parts
        .next()
        .ok_or_else(bad)?
        .parse()
        .map_err(|_| bad())?;
    let mm: i64 = clock_parts
        .next()
        .ok_or_else(bad)?
        .parse()
        .map_err(|_| bad())?;
    let ss: i64 = clock_parts
        .next()
        .ok_or_else(bad)?
        .parse()
        .map_err(|_| bad())?;
    let micros: i64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<6}")[..6].parse().map_err(|_| bad())?
    };
    let local_seconds = days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss;
    let us = (local_seconds - i64::from(offset_s)) * 1_000_000 + micros;
    Ok(Instant {
        raw: raw.to_owned(),
        us,
        offset_s,
        local: (y, m, d),
        local_time: if micros == 0 {
            format!("{hh:02}:{mm:02}:{ss:02}")
        } else {
            format!("{hh:02}:{mm:02}:{ss:02}.{micros:06}")
        },
    })
}

impl Instant {
    fn offset_text(&self) -> String {
        let sign = if self.offset_s < 0 { '-' } else { '+' };
        let abs = self.offset_s.unsigned_abs();
        format!("{sign}{:02}:{:02}", abs / 3600, abs % 3600 / 60)
    }
    /// The engine's `ts` object (v1_request.timestamp_fields).
    fn ts(&self) -> Value {
        let (y, m, d) = self.local;
        let date = format!("{y:04}-{m:02}-{d:02}");
        json!({
            "raw": self.raw,
            "us": self.us,
            "offset_s": self.offset_s,
            "date": date,
            "year": y,
            "vienna_date": null,
            "display": format!("{date} {}{}", self.local_time, self.offset_text()),
            "iso": format!("{date}T{}{}", self.local_time, self.offset_text()),
        })
    }
    fn utc_date(&self) -> String {
        let (y, m, d) = civil_from_days(self.us.div_euclid(86_400_000_000));
        format!("{y:04}-{m:02}-{d:02}")
    }
}

#[test]
fn instants_match_pythons_reading() {
    let t = parse_instant("2024-06-17T22:00:00-05:00").unwrap();
    assert_eq!(t.utc_date(), "2024-06-18");
    assert_eq!(t.ts()["display"], "2024-06-17 22:00:00-05:00");
    assert_eq!(t.ts()["iso"], "2024-06-17T22:00:00-05:00");
    let z = parse_instant("1970-01-01T00:00:01Z").unwrap();
    assert_eq!((z.us, z.offset_s), (1_000_000, 0));
    assert!(parse_instant("2024-01-01T00:00:00").is_err());
}

// ------------------------------------------------------------ the shape

const DOCUMENT_KEYS: [&str; 10] = [
    "schema",
    "country",
    "registry_value",
    "spec",
    "status",
    "retrieved",
    "currency",
    "timezone",
    "tax_year",
    "vectors",
];

fn check_shape(code: &str, doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for key in DOCUMENT_KEYS {
        if doc.get(key).is_none() {
            out.push(format!("{code}: document lacks {key}"));
        }
    }
    if doc["schema"] != "kassiber.tax-country-vectors/1" {
        out.push(format!("{code}: schema {}", doc["schema"]));
    }
    if doc["registry_value"] != code {
        out.push(format!("{code}: registry_value {}", doc["registry_value"]));
    }
    let mut ids = BTreeSet::new();
    for vector in doc["vectors"].as_array().into_iter().flatten() {
        let id = vector["id"].as_str().unwrap_or("?").to_owned();
        if !ids.insert(id.clone()) {
            out.push(format!("{code}: {id} twice"));
        }
        for key in ["title", "rules", "policy", "events", "expected"] {
            if vector.get(key).is_none() {
                out.push(format!("{id}: lacks {key}"));
            }
        }
        if !vector["policy"]["method"].is_string()
            || !matches!(
                vector["policy"]["pool_scope"].as_str(),
                Some("global" | "wallet")
            )
        {
            out.push(format!(
                "{id}: policy needs a method and a pool_scope of global or wallet"
            ));
        }
        for event in vector["events"].as_array().into_iter().flatten() {
            for key in ["id", "time", "type", "wallet"] {
                if !event[key].is_string() {
                    out.push(format!("{id}: an event lacks {key}"));
                }
            }
            if let Some(time) = event["time"].as_str() {
                if let Err(error) = parse_instant(time) {
                    out.push(format!("{id}: {error}"));
                }
            }
        }
    }
    out
}

// --------------------------------------------------------- United States

fn dec(text: &str) -> Decimal {
    text.parse().unwrap_or_else(|_| panic!("decimal {text:?}"))
}
fn field(event: &Value, key: &str) -> Option<Decimal> {
    event[key].as_str().map(dec)
}
fn canonical(d: &Decimal) -> Value {
    json!(d.to_canonical())
}
fn money(d: &Decimal) -> String {
    us_tax::cents(d).unwrap()
}
/// A quantity as the reference renders it: at most 11 places, no
/// trailing zeros.
fn qty(d: &Decimal) -> String {
    let text = d.to_fixed_places(11, Rounding::HalfEven).unwrap();
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    if text == "-0" {
        "0".into()
    } else {
        text
    }
}

const INCOME_TYPES: [(&str, &str); 5] = [
    ("staking", "STAKING"),
    ("mining", "MINING"),
    ("interest", "INTEREST"),
    ("income", "INCOME"),
    ("airdrop_service", "AIRDROP"),
];

/// The engine entries of a vector's events, by row (1-based, input order).
fn us_entries(vector: &Value) -> Result<Vec<Value>, (String, String)> {
    let mut entries = Vec::new();
    for (index, event) in vector["events"].as_array().unwrap().iter().enumerate() {
        let id = event["id"].as_str().unwrap();
        let ts = parse_instant(event["time"].as_str().unwrap()).unwrap().ts();
        let wallet = event["wallet"].as_str().unwrap();
        let quantity = field(event, "quantity").unwrap();
        let row = index as i64 + 1;
        let base =
            json!({"row": row, "unique_id": id, "asset": event["asset"], "notes": null, "ts": ts});
        let mut entry = base.as_object().unwrap().clone();
        let mut put = |key: &str, value: Value| {
            entry.insert(key.into(), value);
        };
        match event["type"].as_str().unwrap() {
            kind @ ("buy" | "income" | "airdrop") => {
                let fiat = field(event, "fiat").unwrap();
                let type_ = if kind == "buy" {
                    "BUY"
                } else {
                    let subkind = event["subkind"].as_str().unwrap_or("");
                    INCOME_TYPES
                        .iter()
                        .find(|(name, _)| *name == subkind)
                        .map(|(_, t)| *t)
                        .ok_or_else(|| ("income_kind_unsupported".to_owned(), id.to_owned()))?
                };
                put("kind", json!("in"));
                put("transaction_type", json!(type_));
                put("exchange", json!(wallet));
                put("holder", json!("Holder"));
                put("spot_price", canonical(&fiat.div(&quantity, &CTX).unwrap()));
                put("crypto_in", canonical(&quantity));
                put("fiat_in_no_fee", canonical(&fiat));
                put("fiat_in_with_fee", canonical(&fiat));
                put("fiat_fee", json!("0E0"));
            }
            "sell" | "spend" => {
                let fiat = field(event, "fiat").unwrap();
                let fee = field(event, "fee_quantity").unwrap_or_else(Decimal::zero);
                let spot = fiat.div(&quantity, &CTX).unwrap();
                put("kind", json!("out"));
                put("transaction_type", json!("SELL"));
                put("exchange", json!(wallet));
                put("holder", json!("Holder"));
                put("spot_price", canonical(&spot));
                put("crypto_out_no_fee", canonical(&quantity));
                put("crypto_fee", canonical(&fee));
                put("fiat_out_no_fee", canonical(&fiat));
                put("fiat_fee", canonical(&fee.mul(&spot, &CTX).unwrap()));
            }
            "transfer" => {
                let fee = field(event, "fee_quantity").unwrap_or_else(Decimal::zero);
                let spot = match field(event, "fee_fiat_value") {
                    Some(value) if !fee.is_zero() => value.div(&fee, &CTX).unwrap(),
                    _ => Decimal::zero(),
                };
                put("kind", json!("intra"));
                put("from_exchange", json!(wallet));
                put("from_holder", json!("Holder"));
                put("to_exchange", event["to_wallet"].clone());
                put("to_holder", json!("Holder"));
                put("spot_price", canonical(&spot));
                put("crypto_sent", canonical(&quantity.add(&fee, &CTX).unwrap()));
                put("crypto_received", canonical(&quantity));
            }
            other => return Err((format!("unsupported_event_{other}"), id.to_owned())),
        }
        entries.push(Value::Object(entry));
    }
    Ok(entries)
}

fn us_refusal(message: &str) -> &'static str {
    if message.contains("per wallet from 2025") {
        "wallet_scope_required"
    } else if message.starts_with(LOTS_EXHAUSTED) {
        "insufficient_lots"
    } else {
        "engine_error"
    }
}

fn us_compute(vector: &Value) -> Value {
    let refusal = |code: &str, event: &str| json!({"refusal": {"code": code, "event": event}});
    let entries = match us_entries(vector) {
        Ok(entries) => entries,
        Err((code, event)) => return refusal(&code, &event),
    };
    let events: Vec<&Value> = vector["events"].as_array().unwrap().iter().collect();
    let request = json!({
        "schema_version": 1,
        "operation": "compute",
        "country": {"kind": "us"},
        "method": vector["policy"]["method"],
        "pool_scope": vector["policy"]["pool_scope"],
        "assets": [{"asset": "BTC", "entries": entries}],
    });
    let response: Value =
        serde_json::from_str(&kassiber_tax_core::compute(&request.to_string())).unwrap();
    if response["ok"] != true {
        let message = response["error"]["message"].as_str().unwrap_or("");
        return refusal(us_refusal(message), "-");
    }
    let asset = &response["assets"][0];
    let event_of = |row: i64| events[(row - 1) as usize];
    let date_of = |row: i64| {
        parse_instant(event_of(row)["time"].as_str().unwrap())
            .unwrap()
            .utc_date()
    };

    // Group the gain/losses by event, in the engine's order.
    let mut order: Vec<i64> = Vec::new();
    let mut by_event: BTreeMap<i64, Vec<&Value>> = BTreeMap::new();
    for gl in asset["gain_losses"].as_array().unwrap() {
        let row = gl["event"]["row"].as_i64().unwrap();
        if !by_event.contains_key(&row) {
            order.push(row);
        }
        by_event.entry(row).or_default().push(gl);
    }
    let (mut disposals, mut income, mut fees) = (Vec::new(), Vec::new(), Vec::new());
    let (mut rows, mut income_rows, mut fee_rows) =
        (Vec::<Row>::new(), Vec::<Income>::new(), Vec::<Fee>::new());
    let sum = |gls: &[&Value], key: &str| {
        gls.iter().fold(Decimal::zero(), |acc, gl| {
            acc.add(&dec_canonical(&gl[key]), &CTX).unwrap()
        })
    };
    for row in order {
        let event = event_of(row);
        let gls = &by_event[&row];
        let id = event["id"].as_str().unwrap();
        let date = date_of(row);
        let quantity = sum(gls, "crypto_amount");
        match event["type"].as_str().unwrap() {
            "income" | "airdrop" => {
                let value = sum(gls, "proceeds");
                income.push(json!({"event": id, "date": date, "subkind": event["subkind"], "quantity": qty(&quantity), "value": money(&value)}));
                income_rows.push(Income {
                    transaction_id: id.into(),
                    wallet: event["wallet"].as_str().unwrap().into(),
                    asset: "BTC".into(),
                    date,
                    kind: event["subkind"].as_str().unwrap().into(),
                    quantity,
                    value,
                });
            }
            "transfer" => {
                let cost = sum(gls, "fiat_cost_basis");
                fees.push(json!({"event": id, "wallet": event["wallet"], "date": date, "quantity": qty(&quantity), "cost": money(&cost), "value": event["fee_fiat_value"]}));
                fee_rows.push(Fee {
                    transaction_id: id.into(),
                    wallet: event["wallet"].as_str().unwrap().into(),
                    asset: "BTC".into(),
                    date,
                    quantity,
                    cost,
                });
            }
            kind => {
                let mut fragments = Vec::new();
                let mut terms: BTreeMap<bool, Vec<&Value>> = BTreeMap::new();
                for gl in gls {
                    let lot = gl["lot"].as_i64().unwrap();
                    let long = gl["long_term"].as_bool().unwrap();
                    terms.entry(long).or_default().push(gl);
                    fragments.push(json!({
                        "lot": event_of(lot)["id"],
                        "acquired": date_of(lot),
                        "quantity": qty(&dec_canonical(&gl["crypto_amount"])),
                        "cost": money(&dec_canonical(&gl["fiat_cost_basis"])),
                        "proceeds": money(&dec_canonical(&gl["proceeds"])),
                        "gain": money(&dec_canonical(&gl["fiat_gain"])),
                        "term": if long { "long" } else { "short" },
                    }));
                }
                disposals.push(json!({"event": id, "kind": kind, "asset": "BTC", "wallet": event["wallet"], "date": date, "fragments": fragments}));
                for (long, gls) in terms {
                    let lots: BTreeSet<String> = gls
                        .iter()
                        .map(|gl| date_of(gl["lot"].as_i64().unwrap()))
                        .collect();
                    rows.push(Row {
                        transaction_id: id.into(),
                        wallet: event["wallet"].as_str().unwrap().into(),
                        asset: "BTC".into(),
                        quantity: sum(&gls, "crypto_amount"),
                        acquired: if lots.len() == 1 {
                            lots.into_iter().next().unwrap()
                        } else {
                            "VARIOUS".into()
                        },
                        sold: date.clone(),
                        proceeds: sum(&gls, "proceeds"),
                        cost: sum(&gls, "fiat_cost_basis"),
                        gain: sum(&gls, "fiat_gain"),
                        term: if long { Term::Long } else { Term::Short },
                    });
                }
            }
        }
    }
    let mut years = Map::new();
    for year in us_tax::years(&rows, &income_rows, &fee_rows).unwrap() {
        years.insert(year.year.to_string(), us_tax::year_json(&year).unwrap());
    }
    // Holdings: each wallet's own lots under the wallet scope; the book's
    // open lots under the global scope.
    let mut holdings = Map::new();
    let put =
        |holdings: &mut Map<String, Value>, wallet: &str, quantity: &Decimal, cost: &Decimal| {
            holdings.insert(
                wallet.into(),
                json!({"BTC": {"quantity": qty(quantity), "cost": money(cost)}}),
            );
        };
    if let Some(wallets) = asset["wallet_holdings"].as_array() {
        for wallet in wallets {
            put(
                &mut holdings,
                wallet["exchange"].as_str().unwrap(),
                &dec_canonical(&wallet["quantity"]),
                &dec_canonical(&wallet["cost_basis"]),
            );
        }
    } else {
        let one = Decimal::from(1u8);
        let (mut quantity, mut cost) = (Decimal::zero(), Decimal::zero());
        for position in asset["open_positions"].as_array().unwrap() {
            let remaining = one
                .sub(&dec_canonical(&position["sold_percentage"]), &CTX)
                .unwrap();
            let lot = event_of(position["row"].as_i64().unwrap());
            let lot_quantity = field(lot, "quantity").unwrap();
            quantity = quantity
                .add(&lot_quantity.mul(&remaining, &CTX).unwrap(), &CTX)
                .unwrap();
            cost = cost
                .add(
                    &dec_canonical(&position["fiat_in_with_fee"])
                        .mul(&remaining, &CTX)
                        .unwrap(),
                    &CTX,
                )
                .unwrap();
        }
        if quantity.is_positive() {
            put(&mut holdings, "*", &quantity, &cost);
        }
    }
    json!({"disposals": disposals, "income": income, "fees": fees, "years": years, "holdings": holdings})
}

fn dec_canonical(value: &Value) -> Decimal {
    Decimal::from_canonical(value.as_str().unwrap()).unwrap()
}

fn diff(path: &str, expected: &Value, actual: &Value, out: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for key in e.keys().chain(a.keys()).collect::<BTreeSet<_>>() {
                match (e.get(key), a.get(key)) {
                    (Some(e), Some(a)) => diff(&format!("{path}.{key}"), e, a, out),
                    (Some(e), None) => {
                        out.push(format!("{path}.{key}: expected {e}, computed nothing"))
                    }
                    (None, Some(a)) => out.push(format!("{path}.{key}: unexpected {a}")),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                out.push(format!(
                    "{path}: expected {} items, computed {}: {actual}",
                    e.len(),
                    a.len()
                ));
                return;
            }
            for (index, (e, a)) in e.iter().zip(a).enumerate() {
                diff(&format!("{path}[{index}]"), e, a, out);
            }
        }
        _ if expected == actual => {}
        _ => out.push(format!("{path}: expected {expected}, computed {actual}")),
    }
}

/// The countries whose vectors this runner computes.
fn compute(code: &str) -> Option<fn(&Value) -> Value> {
    match code {
        "us" => Some(us_compute),
        "de" => Some(de_compute),
        "ch" => Some(ch_compute),
        "ie" => Some(gb_ie::ie_compute),
        "gb" => Some(gb_ie::gb_compute),
        "es" => Some(es_jp::es_compute),
        "jp" => Some(es_jp::jp_compute),
        _ => None,
    }
}

/// Vectors whose refusal is not the engine's, with the named test that
/// checks it on the product path instead.
const ELSEWHERE: &[(&str, &str)] = &[(
    "de-24",
    "the BTC/LBTC rail decision is custody's: kassiber-custody \
     `a_rail_pair_without_a_decided_policy_is_held_where_the_country_requires_one` and \
     kassiber-transfers `rail_pairs_need_an_explicit_policy_where_the_country_requires_one`",
)];

// --------------------------------------------------------------- Germany

/// Kassiber records a sale's fiat commission netted into its price: a
/// transaction has one fiat value and a fee in the asset, no fiat fee. The
/// specification lists the commission as Werbungskosten (DE-FEE-DISP); in
/// Kassiber's report it is part of the net proceeds instead, with the same
/// gain. The expected figures are restated that way: each fragment's
/// proceeds and Werbungskosten lose its share of the commission by quantity,
/// and so do the year's taxable totals (`docs/reference/tax-country-de.md`,
/// "Differences from the specification").
fn net_fiat_commissions(vector: &Value, expected: &mut Value) {
    let less = |slot: &mut Value, share: &Decimal| {
        let value = dec(slot.as_str().unwrap()).sub(share, &CTX).unwrap();
        *slot = json!(round(&value, Rounding::HalfEven));
    };
    let mut taxable_shares = Vec::new();
    for event in vector["events"].as_array().unwrap() {
        let Some(commission) = field(event, "fee_fiat") else {
            continue;
        };
        if !matches!(event["type"].as_str(), Some("sell" | "spend")) {
            continue;
        }
        let total = field(event, "quantity").unwrap();
        let Some(disposals) = expected["disposals"].as_array_mut() else {
            continue;
        };
        for disposal in disposals.iter_mut().filter(|d| d["event"] == event["id"]) {
            let year = disposal["date"].as_str().unwrap()[..4].to_owned();
            for fragment in disposal["fragments"].as_array_mut().unwrap() {
                let share = commission
                    .mul(&dec(fragment["quantity"].as_str().unwrap()), &CTX)
                    .unwrap()
                    .div(&total, &CTX)
                    .unwrap();
                less(&mut fragment["proceeds"], &share);
                less(&mut fragment["expenses"], &share);
                if fragment["taxable"] == true {
                    taxable_shares.push((year.clone(), share));
                }
            }
        }
    }
    for (year, share) in taxable_shares {
        let block = &mut expected["years"][&year];
        less(&mut block["p23_taxable_proceeds"], &share);
        less(&mut block["p23_taxable_expenses"], &share);
    }
}

/// The engine's `rules` country of a registry entry.
fn registry_rules(code: &str) -> Value {
    kassiber_tax_countries::lookup(code)
        .and_then(|entry| entry.engine_country())
        .expect("a country with engine rules")
        .expect("a calendar holding period")
}

/// A refusal of the response: a held receipt's country code, or a
/// received gift without the donor's data (DE-GIFT-IN).
fn refusal_of(response: &Value) -> Option<Value> {
    response["quarantines"].as_array()?.iter().find_map(|q| {
        let code = match (&q["detail"]["code"], &q["detail"]["valuation_mode"]) {
            (Value::String(code), _) => code.clone(),
            (_, Value::String(mode)) if mode == "donor_basis" => {
                "gift_predecessor_data_required".to_owned()
            }
            _ => return None,
        };
        let row = q["row"].as_str()?.trim_start_matches("custody-tax:");
        Some(json!({"refusal": {"code": code, "event": row}}))
    })
}

/// Germany through the engine under its registry rules and its layer's
/// years (`kassiber_reports::de_tax`).
fn de_compute(vector: &Value) -> Value {
    use kassiber_reports::de_tax;
    let zone = "Europe/Berlin";
    let request = match probe_request(vector, registry_rules("de")) {
        Ok(request) => request,
        Err(reason) => return json!({"refusal": {"code": "not_computable", "event": reason}}),
    };
    let response: Value =
        serde_json::from_str(&kassiber_tax_core::ledger(&request.to_string())).unwrap();
    if response["ok"] != true {
        return json!({"refusal": {"code": "engine_error", "event": response["error"]["message"]}});
    }
    if let Some(refusal) = refusal_of(&response) {
        return refusal;
    }
    let event = |id: &str| {
        vector["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap_or_else(|| panic!("no event {id}"))
            .clone()
    };
    let base = |row: &str| -> String {
        let row = row.trim_start_matches("custody-tax:");
        row.strip_suffix("-out")
            .or(row.strip_suffix("-in"))
            .unwrap_or(row)
            .to_owned()
    };
    let lot_name = |row: &str| -> String {
        let row = row.trim_start_matches("custody-tax:");
        match row.strip_suffix("-in") {
            Some(swap) if event(swap)["type"] == "swap" => format!("{swap}:in"),
            Some(other) => base(other),
            None => row.to_owned(),
        }
    };
    let (mut disposals, mut income): (Vec<(String, de_tax::Disposal)>, Vec<de_tax::Income>) =
        (Vec::new(), Vec::new());
    for entry in response["entries"].as_array().unwrap() {
        let entry_type = entry["entry_type"].as_str().unwrap();
        let Some(row) = entry["row"].as_str() else {
            continue;
        };
        let id = base(row);
        let source = event(&id);
        let date = local_date(vector, &id, zone).iso();
        match entry_type {
            "income" => {
                let kind = source["subkind"].as_str().unwrap_or("").to_owned();
                income.push(de_tax::Income {
                    transaction_id: id.clone(),
                    wallet: source["wallet"].as_str().unwrap().into(),
                    asset: source["asset"].as_str().unwrap().into(),
                    date,
                    section: de_tax::section(&kind),
                    kind,
                    quantity: dec_canonical(&entry["quantity"]),
                    value: dec_canonical(&entry["proceeds"]),
                });
            }
            "disposal" | "fee" | "transfer_fee" => {
                let (name, kind) = if entry_type == "disposal" {
                    (id.clone(), source["type"].as_str().unwrap().to_owned())
                } else {
                    (format!("{id}:fee"), "fee".to_owned())
                };
                let taxable = entry["capital_gains_type"] != "long";
                let fragments = entry["fragments"].as_array().unwrap().iter().map(|f| {
                    let lot = lot_name(f["lot"]["row"].as_str().unwrap());
                    let acquired = local_date(vector, lot.trim_end_matches(":in"), zone).iso();
                    de_tax::Fragment {
                        lot,
                        acquired,
                        quantity: dec_canonical(&f["crypto_amount"]),
                        cost: dec_canonical(&f["cost_basis"]),
                        proceeds: dec_canonical(&f["proceeds"]),
                        expenses: f.get("expenses").map_or_else(Decimal::zero, dec_canonical),
                        gain: dec_canonical(&f["gain_loss"]),
                        taxable,
                    }
                });
                match disposals.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, disposal)) => disposal.fragments.extend(fragments),
                    None => disposals.push((
                        name,
                        de_tax::Disposal {
                            transaction_id: id.clone(),
                            wallet: source["wallet"].as_str().unwrap().into(),
                            asset: entry["asset"].as_str().unwrap().into(),
                            kind,
                            date,
                            fragments: fragments.collect(),
                        },
                    )),
                }
            }
            _ => {}
        }
    }
    let mut years = Map::new();
    let typed: Vec<de_tax::Disposal> = disposals.iter().map(|(_, d)| d.clone()).collect();
    for year in de_tax::years(&typed, &income).unwrap() {
        years.insert(year.year.to_string(), de_tax::year_json(&year).unwrap());
    }
    let money = |d: &Decimal| de_tax::cents(d).unwrap();
    let mut holdings = Map::new();
    for holding in response["holdings"].as_array().unwrap() {
        let wallet = holding["wallet"].as_str().unwrap();
        let slot = holdings
            .entry(wallet.to_owned())
            .or_insert_with(|| json!({}));
        slot[holding["asset"].as_str().unwrap()] = json!({
            "quantity": qty(&dec_canonical(&holding["quantity"])),
            "cost": money(&dec_canonical(&holding["cost_basis"])),
        });
    }
    json!({
        "disposals": disposals.iter().map(|(name, d)| json!({
            "event": name, "kind": d.kind, "asset": d.asset, "wallet": d.wallet, "date": d.date,
            "fragments": d.fragments.iter().map(|f| json!({
                "lot": f.lot, "acquired": f.acquired, "quantity": qty(&f.quantity),
                "cost": money(&f.cost), "proceeds": money(&f.proceeds),
                "expenses": money(&f.expenses), "gain": money(&f.gain), "taxable": f.taxable,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "income": income.iter().map(|i| json!({
            "event": i.transaction_id, "date": i.date, "subkind": i.kind,
            "quantity": qty(&i.quantity), "value": money(&i.value), "section": i.section,
        })).collect::<Vec<_>>(),
        "years": years,
        "holdings": holdings,
    })
}

#[test]
fn every_country_file_has_the_shared_shape_and_listed_countries_compute() {
    let files = files();
    let codes: Vec<&str> = files.iter().map(|(code, _)| code.as_str()).collect();
    // The research files and the US file are all here.
    for code in ["ch", "de", "es", "gb", "ie", "jp", "us"] {
        assert!(
            codes.contains(&code),
            "missing tax-country-{code}-vectors.json"
        );
    }
    let mut failures = Vec::new();
    let mut computed = 0;
    let mut per_country: BTreeMap<&str, usize> = BTreeMap::new();
    for (code, doc) in &files {
        failures.extend(check_shape(code, doc));
        let Some(entry) = kassiber_tax_countries::lookup(code) else {
            continue;
        };
        assert_eq!(doc["timezone"], entry.timezone, "{code}: time zone");
        let Some(run) = compute(code) else {
            failures.push(format!(
                "{code} is in the registry but the runner cannot compute it"
            ));
            continue;
        };
        for vector in doc["vectors"].as_array().unwrap() {
            let id = vector["id"].as_str().unwrap();
            if let Some((_, test)) = ELSEWHERE.iter().find(|(prefix, _)| id.starts_with(prefix)) {
                eprintln!("{id}: checked by {test}");
                continue;
            }
            let mut actual = run(vector);
            let mut expected = vector["expected"].clone();
            if code == "de" {
                net_fiat_commissions(vector, &mut expected);
            }
            if code == "jp" {
                es_jp::net_jp_commissions(vector, &mut expected);
            }
            // A refusal must name the code; the event is the reference's.
            if let Some(refusal) = expected.get_mut("refusal") {
                refusal
                    .as_object_mut()
                    .unwrap()
                    .retain(|key, _| key == "code");
                if let Some(refusal) = actual.get_mut("refusal") {
                    refusal
                        .as_object_mut()
                        .unwrap()
                        .retain(|key, _| key == "code");
                }
            }
            diff(id, &expected, &actual, &mut failures);
            computed += 1;
            *per_country.entry(code.as_str()).or_default() += 1;
        }
    }
    assert!(computed >= 20, "only {computed} vectors computed");
    // Every vector of Spain and Japan runs through its registry entry.
    for (code, all) in [("es", 17), ("jp", 19)] {
        assert_eq!(per_country.get(code).copied(), Some(all), "{code}");
    }
    assert!(
        failures.is_empty(),
        "{} problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ----------------------------------------------------------- Switzerland

/// Switzerland through the engine under its registry rules and its layer
/// (`kassiber_reports::ch_tax`): disposals and income are the engine's;
/// the year-end holdings follow the events, valued by the vector's
/// `valuations`; the KS 36 inputs are the vector's policy.
fn ch_compute(vector: &Value) -> Value {
    use kassiber_reports::ch_tax;
    let zone = "Europe/Zurich";
    let policy = &vector["policy"];
    if policy["professional_trader"] == true {
        return json!({"refusal": {"code": "professional_trading_unsupported", "event": "-"}});
    }
    let mut rules = registry_rules("ch");
    rules["rail_pegs"] = json!(true);
    let request = match probe_request(vector, rules) {
        Ok(request) => request,
        Err(reason) => return json!({"refusal": {"code": "not_computable", "event": reason}}),
    };
    let response: Value =
        serde_json::from_str(&kassiber_tax_core::ledger(&request.to_string())).unwrap();
    if response["ok"] != true {
        return json!({"refusal": {"code": "engine_error", "event": response["error"]["message"]}});
    }
    if let Some(refusal) = refusal_of(&response) {
        return refusal;
    }
    let events = vector["events"].as_array().unwrap();
    let event = |id: &str| events.iter().find(|e| e["id"] == id).unwrap().clone();
    let base = |row: &str| -> String {
        let row = row.trim_start_matches("custody-tax:");
        row.strip_suffix("-out")
            .or(row.strip_suffix("-in"))
            .unwrap_or(row)
            .to_owned()
    };
    let date_of = |id: &str| local_date(vector, id, zone).iso();
    let mut book = ch_tax::Book::default();
    for entry in response["entries"].as_array().unwrap() {
        let Some(row) = entry["row"].as_str() else {
            continue;
        };
        let id = base(row);
        let source = event(&id);
        match entry["entry_type"].as_str().unwrap() {
            "income" => {
                let kind = match source["subkind"].as_str().unwrap_or("") {
                    "airdrop_unclassified" | "" => "airdrop".to_owned(),
                    other => other.to_owned(),
                };
                book.income.push(ch_tax::Income {
                    transaction_id: id.clone(),
                    wallet: source["wallet"].as_str().unwrap().into(),
                    asset: source["asset"].as_str().unwrap().into(),
                    date: date_of(&id),
                    category: ch_tax::category(&kind),
                    kind,
                    quantity: dec_canonical(&entry["quantity"]),
                    value: dec_canonical(&entry["proceeds"]),
                });
            }
            "disposal" => {
                let date = date_of(&id);
                let fragments = entry["fragments"].as_array().unwrap().iter().map(|f| {
                    let lot = base(f["lot"]["row"].as_str().unwrap());
                    let acquired = date_of(&lot);
                    ch_tax::Fragment {
                        held_6_months: ch_tax::held_6_months(&acquired, &date),
                        lot,
                        acquired,
                        quantity: dec_canonical(&f["crypto_amount"]),
                        cost: dec_canonical(&f["cost_basis"]),
                        proceeds: dec_canonical(&f["proceeds"]),
                        gain: dec_canonical(&f["gain_loss"]),
                    }
                });
                match book.disposals.iter_mut().find(|d| d.transaction_id == id) {
                    Some(disposal) => disposal.fragments.extend(fragments),
                    None => book.disposals.push(ch_tax::Disposal {
                        transaction_id: id.clone(),
                        wallet: source["wallet"].as_str().unwrap().into(),
                        asset: entry["asset"].as_str().unwrap().into(),
                        kind: source["type"].as_str().unwrap().into(),
                        date: date.clone(),
                        fragments: fragments.collect(),
                    }),
                }
            }
            _ => {}
        }
    }
    // Holdings, purchases and the receipts kept out of income, by event.
    let mut moved = |date: &str, wallet: &Value, asset: &Value, quantity: Decimal| {
        book.movements.push(ch_tax::Movement {
            date: date.to_owned(),
            wallet: wallet.as_str().unwrap().into(),
            asset: asset.as_str().unwrap().into(),
            quantity,
        });
    };
    let mut purchases = Vec::new();
    let mut outside = Vec::new();
    for e in events {
        let id = e["id"].as_str().unwrap();
        let date = date_of(id);
        let q = field(e, "quantity").unwrap();
        let negative = |d: &Decimal| Decimal::zero().sub(d, &CTX).unwrap();
        match e["type"].as_str().unwrap() {
            kind @ ("buy" | "income" | "airdrop" | "wages" | "gift_in") => {
                moved(&date, &e["wallet"], &e["asset"], q);
                let value = field(e, "fiat").unwrap();
                match kind {
                    "buy" => purchases.push((
                        date.clone(),
                        value
                            .add(&field(e, "fee_fiat").unwrap_or_else(Decimal::zero), &CTX)
                            .unwrap(),
                    )),
                    "wages" | "gift_in" => outside.push(ch_tax::Outside {
                        transaction_id: id.into(),
                        date: date.clone(),
                        kind: if kind == "wages" { "wages" } else { "gift_in" },
                        value,
                    }),
                    _ => {}
                }
            }
            "sell" | "spend" => moved(&date, &e["wallet"], &e["asset"], negative(&q)),
            "transfer" => {
                let fee = field(e, "fee_quantity").unwrap_or_else(Decimal::zero);
                moved(
                    &date,
                    &e["wallet"],
                    &e["asset"],
                    negative(&q.add(&fee, &CTX).unwrap()),
                );
                moved(&date, &e["to_wallet"], &e["asset"], q);
            }
            "peg" | "swap" => {
                moved(&date, &e["wallet"], &e["asset"], negative(&q));
                moved(
                    &date,
                    &e["wallet"],
                    &e["in_asset"],
                    field(e, "in_quantity").unwrap(),
                );
            }
            other => panic!("{other} events have no holdings rule here"),
        }
    }
    book.purchases = purchases;
    book.outside = outside;
    // The vector's year-end values and KS 36 inputs (one net income for
    // every year).
    let valuations = vector["valuations"].clone();
    let value = |year: i32, asset: &str| -> Option<ch_tax::Valuation> {
        let v = &valuations[format!("{year}-12-31")][asset];
        v["chf"].as_str().map(|chf| ch_tax::Valuation {
            unit: dec(chf),
            source: v["source"].as_str().unwrap().into(),
        })
    };
    let extra: Vec<i32> = valuations
        .as_object()
        .into_iter()
        .flatten()
        .map(|(day, _)| day[..4].parse().unwrap())
        .collect();
    let mut inputs = ch_tax::Inputs {
        debt_financed_without_cover: policy["declared"]["debt_financed_without_cover"].as_bool(),
        derivatives_only_hedging: policy["declared"]["derivatives_only_hedging"].as_bool(),
        ..ch_tax::Inputs::default()
    };
    if let Some(income) = policy["net_income_chf"].as_str() {
        for year in 1990..2100 {
            inputs.net_income.insert(year, dec(income));
        }
    }
    let mut years = Map::new();
    for year in ch_tax::years(&book, &extra, &value, &inputs).unwrap() {
        years.insert(year.year.to_string(), ch_tax::year_json(&year).unwrap());
    }
    let money = |d: &Decimal| ch_tax::cents(d).unwrap();
    json!({
        "disposals": book.disposals.iter().map(|d| json!({
            "event": d.transaction_id, "kind": d.kind, "asset": d.asset, "date": d.date,
            "fragments": d.fragments.iter().map(|f| json!({
                "lot": f.lot, "acquired": f.acquired, "quantity": qty(&f.quantity),
                "cost": money(&f.cost), "proceeds": money(&f.proceeds), "gain": money(&f.gain),
                "held_6_months": f.held_6_months,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "income": book.income.iter().map(|i| json!({
            "event": i.transaction_id, "date": i.date, "subkind": i.kind, "category": i.category,
            "quantity": qty(&i.quantity), "value_chf": money(&i.value),
        })).collect::<Vec<_>>(),
        "years": years,
    })
}

// ------------------------------------------------- shared engine features
//
// The countries after the United States are not in the registry yet, but
// the engine features their entries will switch on are. Each country's
// vectors run here through the `ledger` operation under the rules its
// specification's registry defaults give (the `rules` country, the method
// and the pool scope), and the shared tables and helpers, and the figures
// the engine owns are compared: disposal fragments (lot, quantity, cost,
// gain, holding period), holdings, quarantines, and the year figures the
// shared tables decide. What only the country layer decides (form lines,
// income categories, flags, classifications) is the country worker's, and
// a vector that needs one of those, or a method not implemented yet (the
// UK's share matching, Japan's averages), is listed with the reason it is
// not computed. Once a country's entry exists, its rules come from the
// registry instead of `PROBES`.

use kassiber_reports::tax_years::{exemption_limit, Limit};
use kassiber_tax_countries::calendar::{self, Date};
use kassiber_tax_countries::{tables, years};

/// Each probed country: its engine rules, its swap in-leg naming, and the
/// vectors it does not compute, with why.
struct Probe {
    code: &'static str,
    rules: fn() -> Value,
    /// How the vectors name the lot a swap acquired (`x1:in` or `x1`).
    swap_lot: fn(&str) -> String,
    skipped: &'static [(&'static str, &'static str)],
}

const PROBES: &[Probe] = &[
    Probe {
        code: "de",
        rules: || registry_rules("de"),
        swap_lot: |id| format!("{id}:in"),
        skipped: &[("de-24", "the rail decision is custody's, not the engine's")],
    },
    Probe {
        code: "ch",
        rules: || {
            // A peg keeps the book's carrying-value default (CH-RAIL).
            let mut rules = registry_rules("ch");
            rules["rail_pegs"] = json!(true);
            rules
        },
        swap_lot: |id| id.to_owned(),
        skipped: &[
            (
                "ch-19",
                "a refusal: the full computation checks the hard fork's code",
            ),
            (
                "ch-20",
                "a refusal: the full computation checks professional trading",
            ),
        ],
    },
];

fn round(d: &Decimal, rounding: Rounding) -> String {
    d.to_fixed_places(2, rounding).unwrap()
}

/// msat of a quantity, which the vectors keep to multiples of 1E-11.
fn msat_of(quantity: &str) -> u64 {
    let (whole, fraction) = quantity.split_once('.').unwrap_or((quantity, ""));
    assert!(fraction.len() <= 11, "{quantity} is finer than msat");
    whole.parse::<u64>().unwrap() * 100_000_000_000
        + format!("{fraction:0<11}").parse::<u64>().unwrap()
}

/// One projection row of a vector event.
fn probe_row(
    id: &str,
    asset: &Value,
    wallet: &str,
    direction: &str,
    time: &str,
    quantity: &str,
    kind: &str,
) -> Value {
    let instant = parse_instant(time).unwrap();
    // A move's or swap's legs name their transaction.
    let anchor = id
        .strip_suffix("-out")
        .or(id.strip_suffix("-in"))
        .unwrap_or(id);
    json!({
        "id": format!("custody-tax:{id}"), "anchor": anchor, "asset": asset, "wallet_id": wallet,
        "direction": direction, "ts": instant.ts(), "cutoff_instant": {"us": instant.us},
        "amount_msat": msat_of(quantity), "fee_msat": 0, "rate": null, "value": null,
        "coarse": false, "kind": kind, "text": id, "regime_override": null,
        "category_override": null, "taxability_off": false, "privacy_hop": false,
        "hold": null, "chain_order": null,
    })
}

const PROBE_INCOME: [(&str, &str); 8] = [
    ("staking", "staking"),
    ("mining", "mining"),
    ("interest", "interest"),
    ("lending_interest", "lending_interest"),
    ("routing_income", "routing_income"),
    ("airdrop", "airdrop"),
    ("airdrop_service", "airdrop"),
    ("hardfork", "hardfork"),
];

/// The ledger request of a vector, or why the engine cannot take it.
fn probe_request(vector: &Value, mut rules: Value) -> Result<Value, String> {
    // The runner's own switch: book a peg as the book's default
    // carrying-value rail pair (a country without a rail decision).
    let pegs = rules
        .as_object_mut()
        .and_then(|rules| rules.remove("rail_pegs"))
        .unwrap_or(Value::Null);
    let rules_for_events = json!({"rail_pegs": pegs});
    // A country whose rules value a sale below market value reads the
    // market value the vector records.
    let below_market = rules.get("below_market").is_some();
    let (mut rows, mut moves, mut cross) = (Vec::new(), Vec::new(), Vec::new());
    let (mut assets, mut wallets) = (BTreeSet::new(), BTreeSet::new());
    let canonical_of = |d: Decimal| json!(d.to_canonical());
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
        let row = |direction: &str, kind: &str| {
            probe_row(id, asset, wallet, direction, time, quantity, kind)
        };
        match event["type"].as_str().unwrap() {
            "buy" | "wages" => {
                let mut r = row("inbound", "buy");
                r["value"] = canonical_of(fiat.unwrap().add(&fee_fiat, &CTX).unwrap());
                rows.push(r);
            }
            kind @ ("sell" | "spend" | "gift_out" | "lost") => {
                if event
                    .get("market_value")
                    .is_some_and(|m| m != &event["fiat"])
                    && kind == "sell"
                    && !below_market
                {
                    return Err("a sale below market value".into());
                }
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
                    // A fiat commission is netted from the price.
                    r["value"] = canonical_of(fiat.sub(&fee_fiat, &CTX).unwrap());
                }
                if let Some(market) = field(event, "market_value").filter(|_| below_market) {
                    r["market_value"] = canonical_of(market);
                }
                if let Some(fee) = event["fee_quantity"].as_str() {
                    r["fee_msat"] = json!(msat_of(fee));
                }
                rows.push(r);
            }
            kind @ ("income" | "airdrop") => {
                let subkind = event["subkind"].as_str().unwrap_or("");
                let kind = match PROBE_INCOME.iter().find(|(name, _)| *name == subkind) {
                    Some((_, kind)) => *kind,
                    // Kassiber has one airdrop kind: an unclassified one.
                    None if kind == "airdrop" => "airdrop",
                    None => return Err(format!("income kind {subkind:?} is the country layer's")),
                };
                let mut r = row("inbound", kind);
                // A fork's coins may come unpriced.
                if let Some(fiat) = fiat {
                    r["value"] = canonical_of(fiat);
                }
                rows.push(r);
            }
            "hardfork" => {
                let mut r = row("inbound", "hardfork");
                if let Some(fiat) = fiat {
                    r["value"] = canonical_of(fiat);
                }
                rows.push(r);
            }
            "gift_in" => {
                let mut r = row("inbound", "gift");
                r["value"] = canonical_of(fiat.unwrap());
                rows.push(r);
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
                rows.extend([out, inbound]);
                moves.push(json!({"out": format!("custody-tax:{id}-out"), "in": format!("custody-tax:{id}-in"),
                                  "source": "reviewed_custody_component", "group_id": null}));
            }
            "swap" => {
                let in_asset = &event["in_asset"];
                assets.insert(in_asset.as_str().unwrap().to_owned());
                let mut out = probe_row(
                    &format!("{id}-out"),
                    asset,
                    wallet,
                    "outbound",
                    time,
                    quantity,
                    "swap",
                );
                // An unpriced leg takes the other leg's value under the
                // country's swap values (jp-03).
                if let Some(fiat) = fiat {
                    out["value"] = canonical_of(fiat);
                }
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
                    inbound["value"] = canonical_of(value);
                }
                rows.extend([out, inbound]);
                cross.push(
                    json!({"pair_id": id, "out_id": format!("custody-tax:{id}-out"),
                                  "in_id": format!("custody-tax:{id}-in"), "out_transaction_id": id,
                                  "in_transaction_id": id, "out_asset": asset, "in_asset": in_asset,
                                  "policy": "taxable", "bitcoin_rail": false}),
                );
            }
            "peg" if rules_for_events["rail_pegs"] == true => {
                let in_asset = &event["in_asset"];
                assets.insert(in_asset.as_str().unwrap().to_owned());
                let out = probe_row(
                    &format!("{id}-out"),
                    asset,
                    wallet,
                    "outbound",
                    time,
                    quantity,
                    "swap",
                );
                let inbound = probe_row(
                    &format!("{id}-in"),
                    in_asset,
                    wallet,
                    "inbound",
                    time,
                    event["in_quantity"].as_str().unwrap(),
                    "swap",
                );
                rows.extend([out, inbound]);
                cross.push(
                    json!({"pair_id": id, "out_id": format!("custody-tax:{id}-out"),
                                  "in_id": format!("custody-tax:{id}-in"), "out_transaction_id": id,
                                  "in_transaction_id": id, "out_asset": asset, "in_asset": in_asset,
                                  "policy": "carrying-value", "bitcoin_rail": true}),
                );
            }
            other => return Err(format!("{other} events are not the engine's")),
        }
    }
    let method = match vector["policy"]["method"].as_str().unwrap() {
        "average" => "moving_average_fifo_period",
        other => other,
    };
    let mut policy = json!({"country": rules, "method": method, "method_label": method.to_uppercase(),
                            "require_coarse_review": false});
    if vector["policy"]["pool_scope"] == "wallet" {
        policy["pool_scope"] = json!("wallet");
    }
    Ok(json!({
        "schema_version": 2, "operation": "ledger", "policy": policy,
        "config": {"assets": assets, "exchanges": wallets, "holders": ["H"]},
        "holder": "H",
        "wallets": wallets.iter().map(|w| json!({"id": w, "label": w})).collect::<Vec<_>>(),
        "rows": rows, "intra_pairs": moves, "cross_asset_pairs": cross,
    }))
}

/// One computed fragment.
struct Piece {
    lot: String,
    quantity: Decimal,
    cost: Decimal,
    gain: Decimal,
    long_term: bool,
}

/// One disposal's fragments, by the vectors' event name.
struct Disposed {
    event: String,
    /// The event row's id.
    row: String,
    pieces: Vec<Piece>,
}

/// The engine's realized entries, joined by event in first-appearance
/// order; a sale's split-off fee is `<id>:fee`, a move's fee `fee_event`.
fn disposed(response: &Value, probe: &Probe, fee_event: fn(&str) -> String) -> Vec<Disposed> {
    let base = |row: &str| -> String {
        let row = row.trim_start_matches("custody-tax:");
        row.strip_suffix("-out").unwrap_or(row).to_owned()
    };
    let mut out: Vec<Disposed> = Vec::new();
    for entry in response["entries"].as_array().unwrap() {
        let kind = entry["entry_type"].as_str().unwrap();
        if !matches!(kind, "disposal" | "fee" | "transfer_fee") {
            continue;
        }
        let row = base(entry["row"].as_str().unwrap());
        let event = match kind {
            "fee" => format!("{row}:fee"),
            "transfer_fee" => fee_event(&row),
            _ => row.clone(),
        };
        let position = match out.iter().position(|d| d.event == event) {
            Some(position) => position,
            None => {
                out.push(Disposed {
                    event: event.clone(),
                    row: row.clone(),
                    pieces: Vec::new(),
                });
                out.len() - 1
            }
        };
        let disposed = &mut out[position];
        for fragment in entry["fragments"].as_array().unwrap() {
            let lot = fragment["lot"]["row"]
                .as_str()
                .unwrap()
                .trim_start_matches("custody-tax:");
            let lot = match lot.strip_suffix("-in") {
                Some(swap) if !swap.is_empty() => (probe.swap_lot)(swap),
                _ => lot.to_owned(),
            };
            disposed.pieces.push(Piece {
                lot,
                quantity: dec_canonical(&fragment["crypto_amount"]),
                cost: dec_canonical(&fragment["cost_basis"]),
                gain: dec_canonical(&fragment["gain_loss"]),
                long_term: entry["capital_gains_type"] == "long",
            });
        }
    }
    out
}

/// The local date of a vector event in `zone`.
fn local_date(vector: &Value, event: &str, zone: &str) -> Date {
    let event = vector["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == event)
        .unwrap_or_else(|| panic!("no event {event}"));
    let us = parse_instant(event["time"].as_str().unwrap()).unwrap().us;
    calendar::zone(zone).unwrap().local_date(us).unwrap()
}

fn sum<'a>(values: impl Iterator<Item = &'a Decimal>) -> Decimal {
    values.fold(Decimal::zero(), |acc, d| acc.add(d, &CTX).unwrap())
}

/// The rows the response quarantines, by vector event name.
fn quarantined(response: &Value) -> BTreeSet<String> {
    response["quarantines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            let row = q["row"]
                .as_str()
                .unwrap()
                .trim_start_matches("custody-tax:");
            row.strip_suffix("-out")
                .or(row.strip_suffix("-in"))
                .unwrap_or(row)
                .to_owned()
        })
        .collect()
}

/// What a vector's probe found.
enum Outcome {
    Pass,
    Skip(String),
    Fail(Vec<String>),
}

fn run_probe(probe: &Probe, vector: &Value) -> (Value, Result<Value, String>) {
    let request = probe_request(vector, (probe.rules)());
    let response = request.map(|request| {
        serde_json::from_str::<Value>(&kassiber_tax_core::ledger(&request.to_string())).unwrap()
    });
    (vector["expected"].clone(), response)
}

fn probe_de(probe: &Probe, vector: &Value) -> Outcome {
    let (expected, response) = run_probe(probe, vector);
    let response = match response {
        Ok(response) => response,
        Err(reason) => return Outcome::Skip(reason),
    };
    let mut failures = Vec::new();
    if let Some(refusal) = expected.get("refusal") {
        // de-23, de-25, de-26: the receipt is held with the country's code,
        // nothing booked after it as if it were known.
        let event = refusal["event"].as_str().unwrap();
        let held = refusal_of(&response);
        if !quarantined(&response).contains(event)
            || held.as_ref().map(|h| &h["refusal"]["code"]) != Some(&refusal["code"])
        {
            failures.push(format!(
                "{event} is not held as {}: {response}",
                refusal["code"]
            ));
        }
        return if failures.is_empty() {
            Outcome::Pass
        } else {
            Outcome::Fail(failures)
        };
    }
    if response["ok"] != true {
        return Outcome::Fail(vec![format!("engine: {response}")]);
    }
    let zone = "Europe/Berlin";
    let computed = disposed(&response, probe, |row| format!("{row}:fee"));
    let actual: Vec<Value> = computed.iter().map(|d| json!({
        "event": d.event,
        "fragments": d.pieces.iter().map(|p| json!({
            "lot": p.lot, "acquired": local_date(vector, p.lot.trim_end_matches(":in"), zone).iso(),
            "quantity": qty(&p.quantity), "cost": round(&p.cost, Rounding::HalfEven),
            "gain": round(&p.gain, Rounding::HalfEven), "taxable": !p.long_term,
        })).collect::<Vec<_>>(),
    })).collect();
    let wanted: Vec<Value> = expected["disposals"].as_array().unwrap().iter().map(|d| json!({
        "event": d["event"],
        "fragments": d["fragments"].as_array().unwrap().iter().map(|f| json!({
            "lot": f["lot"], "acquired": f["acquired"], "quantity": f["quantity"], "cost": f["cost"],
            "gain": f["gain"], "taxable": f["taxable"],
        })).collect::<Vec<_>>(),
    })).collect();
    diff(
        &format!("{}.disposals", vector["id"]),
        &json!(wanted),
        &json!(actual),
        &mut failures,
    );
    // § 23 by Berlin year: the taxable fragments' net gain against the
    // year's Freigrenze, and the gains outside the period.
    for (year, block) in expected["years"].as_object().unwrap() {
        let year: i32 = year.parse().unwrap();
        let in_year: Vec<&Piece> = computed
            .iter()
            .filter(|d| local_date(vector, &d.row, zone).year == year)
            .flat_map(|d| &d.pieces)
            .collect();
        let taxable = sum(in_year.iter().filter(|p| !p.long_term).map(|p| &p.gain));
        let outside = sum(in_year.iter().filter(|p| p.long_term).map(|p| &p.gain));
        let first = Date::new(year, 1, 1).unwrap();
        let limit = match tables::DE_P23_FREIGRENZE.at(first) {
            years::Lookup::Value(limit) => dec(limit),
            other => {
                failures.push(format!("{year}: no Freigrenze ({other:?})"));
                continue;
            }
        };
        let (verdict, amount) = exemption_limit(&taxable, &limit);
        let actual = json!({
            "p23_net_gain": round(&taxable, Rounding::HalfEven),
            "p23_freigrenze": round(&limit, Rounding::HalfEven),
            "p23_result": match verdict { Limit::None => "none", Limit::Loss => "loss", Limit::Exempt => "exempt_freigrenze", Limit::Taxable => "taxable" },
            "p23_amount": round(&amount, Rounding::HalfEven),
            "exempt_gain_outside_period": round(&outside, Rounding::HalfEven),
        });
        let wanted: Map<String, Value> = actual
            .as_object()
            .unwrap()
            .keys()
            .map(|k| (k.clone(), block[k].clone()))
            .collect();
        diff(
            &format!("{}.years.{year}", vector["id"]),
            &Value::Object(wanted),
            &actual,
            &mut failures,
        );
    }
    check_holdings(vector, &expected, &response, &mut failures);
    if failures.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail(failures)
    }
}

fn check_holdings(vector: &Value, expected: &Value, response: &Value, failures: &mut Vec<String>) {
    let mut actual = Map::new();
    for holding in response["holdings"].as_array().unwrap() {
        let wallet = holding["wallet"].as_str().unwrap();
        let entry = actual.entry(wallet.to_owned()).or_insert_with(|| json!({}));
        entry[holding["asset"].as_str().unwrap()] = json!({
            "quantity": qty(&dec_canonical(&holding["quantity"])),
            "cost": round(&dec_canonical(&holding["cost_basis"]), Rounding::HalfEven),
        });
    }
    diff(
        &format!("{}.holdings", vector["id"]),
        &expected["holdings"],
        &Value::Object(actual),
        failures,
    );
}

/// CH-KS36-C1: each fragment's lot, cost and gain, and whether it was held
/// six calendar months (`calendar::Date::add_months`), in Zurich dates.
fn probe_ch(probe: &Probe, vector: &Value) -> Outcome {
    let (expected, response) = run_probe(probe, vector);
    let response = match response {
        Ok(response) => response,
        Err(reason) => return Outcome::Skip(reason),
    };
    if response["ok"] != true {
        return Outcome::Fail(vec![format!("engine: {response}")]);
    }
    let zone = "Europe/Zurich";
    let computed = disposed(&response, probe, str::to_owned);
    let actual: Vec<Value> = computed.iter().map(|d| {
        let sold = local_date(vector, &d.row, zone);
        json!({"event": d.event, "date": sold.iso(), "fragments": d.pieces.iter().map(|p| {
            let acquired = local_date(vector, &p.lot, zone);
            json!({"lot": p.lot, "acquired": acquired.iso(), "quantity": qty(&p.quantity),
                   "cost": round(&p.cost, Rounding::HalfEven), "gain": round(&p.gain, Rounding::HalfEven),
                   "held_6_months": sold >= acquired.add_months(6)})
        }).collect::<Vec<_>>()})
    }).collect();
    let wanted: Vec<Value> = expected["disposals"].as_array().unwrap().iter().map(|d| json!({
        "event": d["event"], "date": d["date"],
        "fragments": d["fragments"].as_array().unwrap().iter().map(|f| json!({
            "lot": f["lot"], "acquired": f["acquired"], "quantity": f["quantity"], "cost": f["cost"],
            "gain": f["gain"], "held_6_months": f["held_6_months"]})).collect::<Vec<_>>(),
    })).collect();
    let mut failures = Vec::new();
    diff(
        &format!("{}.disposals", vector["id"]),
        &json!(wanted),
        &json!(actual),
        &mut failures,
    );
    if failures.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail(failures)
    }
}

#[test]
fn every_countrys_vectors_exercise_the_shared_engine_features() {
    let mut report = Vec::new();
    let mut failures = Vec::new();
    let mut passed: BTreeMap<String, usize> = BTreeMap::new();
    for (code, doc) in files() {
        let probe = PROBES.iter().find(|probe| probe.code == code);
        for vector in doc["vectors"].as_array().unwrap() {
            let id = vector["id"].as_str().unwrap();
            let skipped =
                probe.and_then(|p| p.skipped.iter().find(|(prefix, _)| id.starts_with(prefix)));
            let outcome = match (code.as_str(), probe, skipped) {
                (_, _, Some((_, reason))) => Outcome::Skip((*reason).to_owned()),
                ("de", Some(p), _) => probe_de(p, vector),
                ("ch", Some(p), _) => probe_ch(p, vector),
                // The United States, Ireland, the United Kingdom, Spain and
                // Japan run through their registry entries above.
                _ => continue,
            };
            match outcome {
                Outcome::Pass => *passed.entry(code.clone()).or_default() += 1,
                Outcome::Skip(reason) => report.push(format!("{id}: not computed: {reason}")),
                Outcome::Fail(mut problems) => failures.append(&mut problems),
            }
        }
    }
    eprintln!(
        "passing vectors per country: {passed:?}\n{}",
        report.join("\n")
    );
    assert!(
        failures.is_empty(),
        "{} problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Every computable vector of each probed country passes (Ireland, the
    // United Kingdom, Spain and Japan run through their registry entries).
    for (code, least) in [("de", 25), ("ch", 18)] {
        assert!(
            passed.get(code).copied().unwrap_or(0) >= least,
            "{code}: {passed:?}"
        );
    }
}
