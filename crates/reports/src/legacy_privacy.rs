use crate::{sql, Result};
use kassiber_envelope::{py_str, AppError};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::{BigInt, Value as Py};
use kassiber_store::rusqlite::{params_from_iter, Connection};
use kassiber_wallets::analysis_features;
use kassiber_wallets::coinjoin_shape::{self, Leg as Shape};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

fn py(v: &Value) -> Py {
    kassiber_pytext::loads(&v.to_string()).expect("JSON value")
}
fn json(v: &Py) -> Result<Value> {
    let raw = kassiber_pyjson::dumps(v, &Default::default())
        .map_err(|_| AppError::new("internal_error", "Invalid privacy-hygiene value"))?;
    serde_json::from_str(&raw)
        .map_err(|_| AppError::new("internal_error", "Invalid privacy-hygiene value"))
}
fn truth(v: &Value) -> bool {
    kassiber_envelope::truthy(&py(v))
}
fn string(v: &Value) -> String {
    py_str(&py(v))
}
fn stripped(v: &Value) -> Option<String> {
    (!v.is_null())
        .then(|| kassiber_pytext::strip(&string(v)).to_owned())
        .filter(|s| !s.is_empty())
}
fn lower(v: &Value) -> String {
    kassiber_pytext::lower(stripped(v).as_deref().unwrap_or(""))
}
fn integer(v: &Value) -> Option<BigInt> {
    match py(v) {
        Py::Int(n) => Some(n),
        Py::Float(n) if n.is_finite() => format!("{:.0}", n.trunc()).parse().ok(),
        Py::Str(n) => kassiber_pytext::parse_int(n.as_str()?).ok(),
        _ => None,
    }
}
fn small(v: &Value) -> i64 {
    integer(v).and_then(|n| n.try_into().ok()).unwrap_or(0)
}
fn floor_sats(n: BigInt) -> BigInt {
    let q = &n / 1000;
    if n < BigInt::from(0) && n % 1000 != BigInt::from(0) {
        q - 1
    } else {
        q
    }
}
fn object(v: &Value) -> Value {
    if v.is_object() {
        return v.clone();
    }
    v.as_str()
        .and_then(|s| kassiber_pytext::loads(s).ok())
        .and_then(|v| json(&v).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}
fn transaction(v: &Value) -> Value {
    let raw = object(v);
    if raw["tx"].is_object() {
        raw["tx"].clone()
    } else {
        raw
    }
}
fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn hex(v: &Value) -> Option<String> {
    let text = lower(v);
    if text.is_empty() || !text.chars().count().is_multiple_of(2) {
        return None;
    }
    let mut high = true;
    for c in text.chars() {
        if c.is_ascii_hexdigit() {
            high = !high;
        } else if !(high && matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')) {
            return None;
        }
    }
    high.then_some(text)
}
fn txid(v: &Value) -> Option<String> {
    hex(v).filter(|s| s.chars().count() == 64)
}
fn script(v: &Value) -> &Value {
    if !v["scriptpubkey"].is_null() {
        &v["scriptpubkey"]
    } else if !v["script_hex"].is_null() {
        &v["script_hex"]
    } else if v["scriptPubKey"].is_object() {
        if truth(&v["scriptPubKey"]["hex"]) {
            &v["scriptPubKey"]["hex"]
        } else {
            &v["scriptPubKey"]["scriptpubkey"]
        }
    } else if v["scriptPubKey"].is_string() {
        &v["scriptPubKey"]
    } else {
        &Value::Null
    }
}
fn address(v: &Value) -> Option<String> {
    let mut value = if truth(&v["scriptpubkey_address"]) {
        &v["scriptpubkey_address"]
    } else {
        &v["address"]
    };
    if (value.is_null() || value == "") && v["scriptPubKey"].is_object() {
        value = &v["scriptPubKey"]["address"];
        if value.is_null() || value == "" {
            value = list(&v["scriptPubKey"]["addresses"])
                .first()
                .unwrap_or(&Value::Null);
        }
    }
    stripped(value)
}
fn kind(address: Option<&str>, script: Option<&str>) -> &'static str {
    let s = script.unwrap_or("");
    if s.starts_with("6a") {
        return "op_return";
    }
    if s.starts_with("5120") && s.len() == 68 {
        return "p2tr";
    }
    if s.starts_with("0014") {
        return "p2wpkh";
    }
    if s.starts_with("0020") {
        return "p2wsh";
    }
    if s.starts_with("a914") && s.ends_with("87") {
        return "p2sh";
    }
    if s.starts_with("76a914") && s.ends_with("88ac") {
        return "p2pkh";
    }
    let a = kassiber_pytext::lower(kassiber_pytext::strip(address.unwrap_or("")));
    if ["bc1p", "tb1p", "bcrt1p"].iter().any(|p| a.starts_with(p)) {
        "p2tr"
    } else if ["bc1q", "tb1q", "bcrt1q"].iter().any(|p| a.starts_with(p)) {
        "segwit"
    } else if ["1", "m", "n"].iter().any(|p| a.starts_with(p)) {
        "p2pkh"
    } else if ["3", "2"].iter().any(|p| a.starts_with(p)) {
        "p2sh"
    } else if ["ex1", "tex1", "el1"].iter().any(|p| a.starts_with(p)) {
        "confidential"
    } else {
        "unknown"
    }
}
fn op_return(v: &Value) -> bool {
    let t = if truth(&v["scriptpubkey_type"]) {
        &v["scriptpubkey_type"]
    } else {
        &v["type"]
    };
    kassiber_pytext::lower(&if truth(t) { string(t) } else { String::new() }) == "op_return"
        || kind(None, hex(script(v)).as_deref()) == "op_return"
}
fn output_value(v: &Value) -> Option<BigInt> {
    let explicit = !v["value_sats"].is_null();
    let v = if explicit {
        &v["value_sats"]
    } else {
        &v["value"]
    };
    if v.is_null() || v.is_boolean() || v == "" {
        return None;
    }
    if explicit || matches!(py(v), Py::Int(_)) || v.as_str().is_some_and(|s| !s.contains('.')) {
        return integer(v);
    }
    let d: Decimal = kassiber_pytext::strip(&string(v)).parse().ok()?;
    d.mul(&Decimal::from(100_000_000), &Context::RP2)
        .ok()?
        .to_fixed_places(0, Rounding::HalfEven)
        .ok()?
        .parse()
        .ok()
}
fn fee_value(v: &Value) -> Option<BigInt> {
    if v.is_null() || v.is_boolean() || v == "" {
        return None;
    }
    if matches!(py(v), Py::Float(_)) || v.as_str().is_some_and(|s| s.contains('.')) {
        let f = if let Some(s) = v.as_str() {
            kassiber_pytext::parse_float(kassiber_pytext::strip(s)).ok()?
        } else {
            v.as_f64()?
        };
        let n = (f * 100_000_000.0).round_ties_even();
        return n
            .is_finite()
            .then(|| format!("{n:.0}").parse().ok())
            .flatten();
    }
    integer(v)
}

type Domain = (String, String);
type Identity = (String, String, String);
fn domain(row: &Value) -> Option<Domain> {
    kassiber_custody::evidence::scope(row)
        .ok()
        .filter(|(chain, _)| matches!(chain.as_str(), "bitcoin" | "liquid"))
}
fn identity(row: &Value) -> Option<Identity> {
    let (chain, network) = domain(row)?;
    let raw = transaction(&row["raw_json"]);
    let r = txid(&raw["txid"]);
    let e = txid(&row["external_id"]);
    if r.is_some() && e.is_some() && r != e {
        return None;
    }
    Some((chain, network, r.or(e)?))
}
#[derive(Clone)]
struct Owner {
    wallet: String,
    sats: BigInt,
    kind: String,
    ambiguous: bool,
}
#[derive(Default)]
struct Inventory {
    outpoints: BTreeMap<(Domain, String, BigInt), Owner>,
    scripts: BTreeMap<(Domain, String), Owner>,
}
fn inventory_index(rows: &[Value]) -> Inventory {
    let mut index = Inventory::default();
    for row in rows {
        let Some(d) =
            domain(&json!({"chain":row["chain"],"network":row["network"],"asset":row["asset"]}))
        else {
            continue;
        };
        let owner = Owner {
            wallet: string(&row["wallet_id"]),
            sats: floor_sats(integer(&row["amount"]).unwrap_or_default()).max(0.into()),
            kind: kind(
                stripped(&row["address"]).as_deref(),
                hex(&row["script_pubkey"]).as_deref(),
            )
            .into(),
            ambiguous: false,
        };
        if let (Some(t), Some(n)) = (
            txid(&row["txid"]),
            integer(&row["vout"]).filter(|n| *n >= 0.into()),
        ) {
            index
                .outpoints
                .entry((d.clone(), t, n))
                .and_modify(|o| o.ambiguous = true)
                .or_insert_with(|| owner.clone());
        }
        if let Some(s) = hex(&row["script_pubkey"]) {
            index
                .scripts
                .entry((d, s))
                .and_modify(|o| {
                    if o.wallet != owner.wallet {
                        if o.kind != owner.kind {
                            o.kind = "unknown".into();
                        }
                        o.wallet.clear();
                        o.sats = 0.into();
                        o.ambiguous = true;
                    }
                })
                .or_insert(owner);
        }
    }
    index
}
#[derive(Default)]
struct Leg {
    index: BigInt,
    outpoint: Option<(String, BigInt)>,
    value: Option<BigInt>,
    /// The value the transaction itself carries, without inventory amounts.
    observed: Option<BigInt>,
    script: Option<String>,
    kind: String,
    owner: Option<Owner>,
    sequence: Option<BigInt>,
    op_return: bool,
}
#[derive(Default)]
struct Parsed {
    support: &'static str,
    reason: Option<&'static str>,
    inputs: Vec<Leg>,
    outputs: Vec<Leg>,
    fee: Option<BigInt>,
    vsize: Option<BigInt>,
}
fn parse(row: &Value, index: &Inventory) -> Parsed {
    let empty = |reason| Parsed {
        support: "none",
        reason: Some(reason),
        ..Default::default()
    };
    let Some(d) = domain(row) else {
        return empty("unsupported_protocol_scope");
    };
    let raw = transaction(&row["raw_json"]);
    let (Some(vin), Some(vout)) = (raw["vin"].as_array(), raw["vout"].as_array()) else {
        return empty("missing_vin_vout");
    };
    let current = txid(&raw["txid"]).or_else(|| txid(&row["external_id"]));
    let mut ambiguous = 0;
    let mut unambiguous = |owner: Option<Owner>| {
        owner.filter(|o| {
            if o.ambiguous {
                ambiguous += 1;
                false
            } else {
                true
            }
        })
    };
    let inputs: Vec<_> = vin
        .iter()
        .filter(|v| v.is_object())
        .map(|v| {
            let outpoint = txid(&v["txid"])
                .filter(|t| t != &"0".repeat(64) && t.bytes().all(|c| c.is_ascii_hexdigit()))
                .zip(
                    if v["vout"].is_boolean() {
                        Some(BigInt::from(v["vout"] == true))
                    } else {
                        integer(&v["vout"])
                    }
                    .filter(|n| *n >= 0.into()),
                );
            let owner = unambiguous(
                outpoint
                    .as_ref()
                    .and_then(|(t, n)| index.outpoints.get(&(d.clone(), t.clone(), n.clone())))
                    .cloned(),
            );
            let observed = output_value(&v["prevout"]).or_else(|| output_value(v));
            let value = observed
                .clone()
                .or_else(|| owner.as_ref().map(|o| o.sats.clone()));
            let mut k = kind(
                address(&v["prevout"]).as_deref(),
                hex(script(&v["prevout"])).as_deref(),
            )
            .to_owned();
            if k == "unknown" {
                if let Some(o) = &owner {
                    k.clone_from(&o.kind);
                }
            }
            Leg {
                outpoint,
                value,
                observed,
                script: hex(script(&v["prevout"])),
                kind: k,
                owner,
                sequence: integer(&v["sequence"]),
                ..Default::default()
            }
        })
        .collect();
    let outputs: Vec<_> = vout
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_object())
        .map(|(i, v)| {
            let n = integer(&v["n"]).unwrap_or_else(|| i.into());
            let s = hex(script(v));
            let mut owner = unambiguous(
                current
                    .as_ref()
                    .and_then(|t| index.outpoints.get(&(d.clone(), t.clone(), n.clone())))
                    .cloned(),
            );
            if owner.is_none() {
                owner = unambiguous(
                    s.as_ref()
                        .and_then(|s| index.scripts.get(&(d.clone(), s.clone())))
                        .cloned(),
                );
            }
            Leg {
                index: n,
                value: output_value(v),
                kind: kind(address(v).as_deref(), s.as_deref()).into(),
                script: s,
                owner,
                op_return: op_return(v),
                ..Default::default()
            }
        })
        .collect();
    if inputs.is_empty() && outputs.is_empty() {
        return empty("empty_vin_vout");
    }
    let mut reason = None;
    if inputs.iter().chain(&outputs).any(|v| v.value.is_none()) {
        reason = Some("missing_prevout_or_output_values");
    }
    if ambiguous > 0 && reason.is_none() {
        reason = Some("ambiguous_inventory_owner");
    }
    let mut fee = fee_value(&raw["fee_sats"]).or_else(|| fee_value(&raw["fee"]));
    if fee.is_none()
        && inputs.iter().all(|v| v.value.is_some())
        && outputs
            .iter()
            .filter(|v| !v.op_return)
            .all(|v| v.value.is_some())
    {
        let computed: BigInt = inputs
            .iter()
            .filter_map(|v| v.value.clone())
            .sum::<BigInt>()
            - outputs
                .iter()
                .filter(|v| !v.op_return)
                .filter_map(|v| v.value.clone())
                .sum::<BigInt>();
        if computed >= 0.into() {
            fee = Some(computed);
        }
    }
    if fee.is_none() {
        fee = integer(&row["fee"]).map(floor_sats);
    }
    Parsed {
        support: if reason.is_none() { "full" } else { "partial" },
        reason,
        inputs,
        outputs,
        fee: fee.map(|n| n.max(0.into())),
        vsize: integer(&raw["vsize"])
            .filter(|n| *n != 0.into())
            .or_else(|| integer(&raw["virtual_size"])),
    }
}

fn boundary(v: &Value) -> Option<String> {
    stripped(v)
        .map(|s| kassiber_pytext::lower(&s).replace('-', "_"))
        .filter(|s| {
            [
                "coinjoin",
                "collaborative",
                "payjoin",
                "payment_in_coinjoin",
                "sweep",
            ]
            .contains(&s.as_str())
        })
}
fn stored_boundary(raw: &Value) -> Option<String> {
    let inner = object(&raw["raw_json"]);
    for payload in [raw, &inner] {
        for key in [
            "privacy_boundary",
            "privacy_boundary_kind",
            "privacyBoundary",
            "privacyBoundaryKind",
            "privacy_hop",
            "privacy_hop_kind",
            "privacyHop",
            "privacyHopKind",
        ] {
            if stripped(&payload[key]).is_some() {
                return Some(boundary(&payload[key]).unwrap_or_else(|| "unknown".into()));
            }
        }
        for key in [
            "islikelycoinjoin",
            "is_likely_coinjoin",
            "isLikelyCoinJoin",
            "likely_coinjoin",
            "likelyCoinJoin",
        ] {
            if payload[key] == true
                || ["1", "true", "yes", "on"].contains(&lower(&payload[key]).as_str())
            {
                return Some("coinjoin".into());
            }
        }
    }
    None
}
fn boundary_level(row: &Value) -> &'static str {
    if ["accepted", "completed", "reviewed"].contains(&lower(&row["review_status"]).as_str()) {
        "reviewed"
    } else {
        "imported"
    }
}
fn coinjoin(
    row: &Value,
    raw: &Value,
    input_count: usize,
    inputs: &[Shape],
    outputs: &[(Shape, bool)],
) -> Option<Value> {
    let outputs: Vec<_> = outputs
        .iter()
        .filter(|(_, op)| !op)
        .map(|(leg, _)| leg.clone())
        .collect();
    let count = coinjoin_shape::most_common(&outputs);
    let b = lower(&row["privacy_boundary"]);
    if b == "coinjoin" || stored_boundary(raw).as_deref() == Some("coinjoin") {
        return Some(
            json!({"pattern":"reviewed_or_imported_coinjoin","participant_count":count.max(1),"impact":if count >= 5 {24} else {18},"evidence_level":if b == "coinjoin" {boundary_level(row)} else {"imported"}}),
        );
    }
    let pattern = coinjoin_shape::shape(input_count, inputs, &outputs)?;
    Some(
        json!({"pattern":pattern,"participant_count":count,"impact":coinjoin_shape::impact(pattern),"evidence_level":"heuristic"}),
    )
}
/// `_coinjoin_signal(row, raw, parsed)`: only values the transaction carries.
fn parsed_coinjoin(row: &Value, raw: &Value, p: &Parsed) -> Option<Value> {
    let shape = |value: &Option<BigInt>, script: &Option<String>| Shape {
        value: value.clone(),
        script: script.clone(),
    };
    let inputs: Vec<_> = p
        .inputs
        .iter()
        .map(|v| shape(&v.observed, &v.script))
        .collect();
    let outputs: Vec<_> = p
        .outputs
        .iter()
        .map(|v| (shape(&v.value, &v.script), v.op_return))
        .collect();
    coinjoin(row, raw, p.inputs.len(), &inputs, &outputs)
}
fn collaboration(row: &Value) -> Option<Value> {
    let raw = object(&row["raw_json"]);
    let marker = &row["privacy_boundary"];
    let b = boundary(marker).or_else(|| {
        if !marker.is_null() && marker != "" {
            Some("unknown".into())
        } else {
            stored_boundary(&raw)
        }
    });
    if b.as_deref() == Some("unknown") {
        return Some(
            json!({"kind":"unknown","source":"invalid_privacy_metadata","evidence_level":"unavailable"}),
        );
    }
    if let Some(b) = b.filter(|s| s != "sweep") {
        return Some(
            json!({"kind":b,"source":"reviewed_or_imported_privacy_boundary","evidence_level":if [json!("accepted"),json!("completed"),json!("reviewed")].contains(&row["review_status"]) {"reviewed"} else {"imported"}}),
        );
    }
    let tx = transaction(&raw);
    let (Some(vin), Some(vout)) = (tx["vin"].as_array(), tx["vout"].as_array()) else {
        return None;
    };
    let inputs: Vec<_> = vin
        .iter()
        .filter(|v| v.is_object())
        .map(|v| Shape {
            value: output_value(&v["prevout"]).or_else(|| output_value(v)),
            script: hex(script(&v["prevout"])),
        })
        .collect();
    let outputs: Vec<_> = vout
        .iter()
        .filter(|v| v.is_object())
        .map(|v| {
            let leg = Shape {
                value: output_value(v),
                script: hex(script(v)),
            };
            (leg, op_return(v))
        })
        .collect();
    coinjoin(&json!({}), &json!({}), vin.len(), &inputs, &outputs).map(
        |signal| json!({"kind":"coinjoin","source":signal["pattern"],"evidence_level":"heuristic"}),
    )
}

fn metadata() -> &'static Value {
    static METADATA: OnceLock<Value> = OnceLock::new();
    METADATA.get_or_init(|| {
        serde_json::from_str(include_str!("legacy_privacy_metadata.json"))
            .expect("finding metadata")
    })
}
fn finding(
    code: &str,
    severity: &str,
    impact: i64,
    scope: &str,
    count: usize,
    details: Value,
) -> Value {
    let metadata = metadata();
    json!({"code":code,"severity":severity,"impact":impact,"scope":scope,"count":count,"details":details,
        "evidence_level":metadata[code]["evidence_level"].as_str().unwrap_or("heuristic"),
        "remediation":metadata[code]["remediation"].as_str().unwrap_or("Review this local tell manually; Kassiber does not mutate accounting state from privacy heuristics."),"attribution":"user_wallet"})
}
fn severity(v: &Value) -> i64 {
    ["positive", "info", "low", "medium", "high", "critical"]
        .iter()
        .position(|s| v == s)
        .unwrap_or(0) as i64
}
fn sort_findings(v: &mut [Value]) {
    v.sort_by_key(|v| {
        (
            -severity(&v["severity"]),
            small(&v["impact"]),
            string(&v["code"]),
        )
    });
}
fn meaningful(legs: &[Leg]) -> BTreeSet<&str> {
    legs.iter()
        .filter(|v| !v.op_return)
        .map(|v| v.kind.as_str())
        .filter(|s| !["", "unknown", "op_return", "fee", "confidential"].contains(s))
        .collect()
}
fn round_amount(n: &BigInt) -> bool {
    *n >= 10_000.into() && n % 100_000 == 0.into()
}
fn transaction_findings(row: &Value, p: &Parsed, collaboration: Option<&Value>) -> Vec<Value> {
    if p.support == "none" {
        return vec![finding(
            "transaction_not_analysable",
            "info",
            0,
            "transaction",
            1,
            json!({"reason":p.reason}),
        )];
    }
    let mut f = Vec::new();
    let raw = object(&row["raw_json"]);
    if let Some(signal) = parsed_coinjoin(row, &raw, p) {
        let mut item = finding(
            "coinjoin_pattern",
            "positive",
            small(&signal["impact"]),
            "transaction",
            small(&signal["participant_count"]) as usize,
            json!({"pattern":signal["pattern"]}),
        );
        item["evidence_level"] = signal["evidence_level"].clone();
        item["attribution"] = json!("local_data");
        f.push(item);
    }
    if lower(&row["privacy_boundary"]) == "payjoin" {
        let mut item = finding(
            "payjoin_boundary",
            "positive",
            8,
            "transaction",
            1,
            json!({}),
        );
        item["evidence_level"] = json!(boundary_level(row));
        item["attribution"] = json!("local_data");
        f.push(item);
    }
    if collaboration.is_some_and(|c| c["kind"] == "unknown") {
        f.push(finding(
            "transaction_coverage_gap",
            "info",
            0,
            "transaction",
            1,
            json!({"reason":"invalid_privacy_metadata"}),
        ));
    }
    let spendable: Vec<_> = p.outputs.iter().filter(|v| !v.op_return).collect();
    if collaboration.is_none() {
        let count = p
            .inputs
            .iter()
            .filter_map(|v| v.outpoint.as_ref())
            .collect::<BTreeSet<_>>()
            .len();
        if count > 1 {
            let (impact, s) = if count >= 50 {
                (-45, "critical")
            } else if count >= 20 {
                (-35, "critical")
            } else if count >= 10 {
                (-25, "high")
            } else if count >= 5 {
                (-15, "high")
            } else {
                (-3 * count as i64, "medium")
            };
            f.push(finding(
                "common_input_ownership",
                s,
                impact,
                "transaction",
                count,
                json!({"input_count":count}),
            ));
        }
        let count = spendable
            .iter()
            .filter(|v| v.value.as_ref().is_some_and(round_amount))
            .count();
        if count > 0 {
            f.push(finding(
                "round_output_amount",
                if count < 3 { "medium" } else { "high" },
                -(count as i64 * 8).min(20),
                "transaction",
                count,
                json!({"round_outputs":count}),
            ));
        }
        let inputs: Vec<_> = p.inputs.iter().filter_map(|v| v.value.as_ref()).collect();
        let known: Vec<_> = spendable.iter().filter_map(|v| v.value.as_ref()).collect();
        if inputs.len() >= 2 && known.len() == 2 {
            let largest = *inputs.iter().max().unwrap();
            if inputs.iter().copied().sum::<BigInt>() > *largest {
                let fee = p.fee.clone().unwrap_or_default();
                let count = known.iter().filter(|n| **n + &fee <= *largest).count();
                if count > 0 {
                    f.push(finding(
                        "unnecessary_input_heuristic",
                        "medium",
                        -8,
                        "transaction",
                        count,
                        json!({"candidate_outputs":count}),
                    ));
                }
            }
        }
    }
    if let (Some(fee), Some(size)) = (&p.fee, p.vsize.as_ref().filter(|n| **n > 0.into())) {
        let rate = fee.to_string().parse::<f64>().unwrap_or(f64::INFINITY)
            / size.to_string().parse::<f64>().unwrap_or(f64::INFINITY);
        let rounded = rate.round_ties_even();
        if (rate - rounded).abs() < 0.01
            && [1., 2., 3., 5., 10., 15., 20., 25., 50., 100.].contains(&rounded)
        {
            f.push(finding(
                "round_fee_rate",
                "low",
                -3,
                "transaction",
                1,
                json!({"sat_vb":rounded as i64}),
            ));
        }
    }
    let count = p
        .inputs
        .iter()
        .filter(|v| {
            v.sequence
                .as_ref()
                .is_some_and(|n| *n < 0xfffffffe_u64.into())
        })
        .count();
    if count > 0 {
        f.push(finding(
            "rbf_signal",
            "low",
            -2,
            "transaction",
            count,
            json!({}),
        ));
    }
    let input_types = meaningful(&p.inputs);
    let output_types = meaningful(&p.outputs);
    let all = input_types.union(&output_types).count();
    if all > 1 {
        f.push(finding(
            "script_type_mix",
            "medium",
            -8,
            "transaction",
            all,
            json!({"type_count":all}),
        ));
    }
    if collaboration.is_none() {
        if spendable.len() == 2
            && input_types.len() == 1
            && output_types.len() > 1
            && spendable
                .iter()
                .filter(|v| input_types.contains(v.kind.as_str()))
                .count()
                == 1
        {
            f.push(finding(
                "change_type_fingerprint",
                "medium",
                -8,
                "transaction",
                1,
                json!({}),
            ));
        }
        let owned: Vec<_> = spendable.iter().filter(|v| v.owner.is_some()).collect();
        if spendable.len() >= 2 && owned.len() == 1 {
            let min = spendable.iter().map(|v| &v.index).min().unwrap();
            let max = spendable.iter().map(|v| &v.index).max().unwrap();
            if owned[0].index == *min || owned[0].index == *max {
                f.push(finding(
                    "change_position_fingerprint",
                    "low",
                    -4,
                    "transaction",
                    1,
                    json!({"position":if owned[0].index == *min {"first"} else {"last"}}),
                ));
            }
        }
    }
    if identity(row).is_some_and(|i| i.0 == "bitcoin") {
        let tx = transaction(&raw);
        let features = analysis_features::normalize(
            &tx[analysis_features::PERSISTED_FEATURE_KEY],
            "stored_transaction",
            None,
        )
        .unwrap_or_else(|| analysis_features::extract(&tx, "raw_transaction", None, &[]));
        for item in analysis_features::evaluate(&features, collaboration) {
            if [
                "unusual_transaction_version",
                "ineffective_absolute_locktime",
                "relative_lock_constraints",
                "consistent_low_r_encoding",
                "nondefault_sighash",
            ]
            .contains(&item["code"].as_str().unwrap_or(""))
            {
                let mut finding = finding(
                    item["code"].as_str().unwrap(),
                    "info",
                    0,
                    "transaction",
                    1,
                    json!({"rule_version":item["rule_version"],"feature_codes":item["feature_codes"],"assumptions":item["assumptions"],"contradictions":item["contradictions"]}),
                );
                finding["evidence_level"] = json!(if item["authority"] == "hypothesis" {
                    "heuristic"
                } else {
                    "ground_truth"
                });
                f.push(finding);
            }
        }
    }
    let count = p.outputs.iter().filter(|v| v.op_return).count();
    if count > 0 {
        f.push(finding(
            "op_return_metadata",
            "high",
            -10,
            "transaction",
            count,
            json!({"outputs":count}),
        ));
    }
    let count = spendable.iter().filter(|v| v.kind == "p2tr").count();
    if count > 0 && count * 2 >= spendable.len() {
        f.push(finding(
            "taproot_usage",
            "positive",
            if count == spendable.len() { 6 } else { 4 },
            "transaction",
            count,
            json!({"taproot_outputs":count}),
        ));
    }
    if lower(&row["direction"]) == "inbound" {
        let metadata = metadata();
        for item in &mut f {
            if small(&item["impact"]) < 0
                && list(&metadata["inbound_counterparty"]).contains(&item["code"])
            {
                item["impact"] = json!(0);
                item["attribution"] = json!("counterparty");
                item["remediation"] = json!("This tell is on the inbound payer side; keep it as context and do not lower the receiving wallet's risk from it.");
            }
        }
    }
    sort_findings(&mut f);
    f
}

fn aggregate(findings: &[Value]) -> Vec<Value> {
    let mut grouped: BTreeMap<String, Value> = BTreeMap::new();
    for item in findings {
        let current = grouped.entry(string(&item["code"])).or_insert_with(|| {
            let mut v = item.clone();
            v["count"] = json!(0);
            v["impact"] = json!(0);
            v["occurrences"] = json!(0);
            v["details"] = json!({});
            v
        });
        for key in ["count", "impact"] {
            current[key] = json!(small(&current[key]) + small(&item[key]));
        }
        current["occurrences"] = json!(small(&current["occurrences"]) + 1);
        if severity(&item["severity"]) > severity(&current["severity"]) {
            for key in ["severity", "evidence_level", "remediation", "attribution"] {
                current[key] = item[key].clone();
            }
        }
        if !truth(&current["details"]) {
            current["details"] = item["details"].clone();
        }
    }
    let mut result: Vec<_> = grouped.into_values().collect();
    result.sort_by_key(|v| {
        (
            -severity(&v["severity"]),
            small(&v["impact"]),
            -small(&v["occurrences"]),
            string(&v["code"]),
        )
    });
    result
}
fn counts(findings: &[Value]) -> Value {
    let mut v = json!({"positive":0,"info":0,"low":0,"medium":0,"high":0,"critical":0});
    for f in findings {
        let key = f["severity"]
            .as_str()
            .filter(|k| !v[k].is_null())
            .unwrap_or("info");
        v[key] = json!(small(&v[key]) + 1);
    }
    v
}
fn risk_level(weight: i64) -> &'static str {
    if weight >= 45 {
        "critical"
    } else if weight >= 25 {
        "high"
    } else if weight >= 10 {
        "medium"
    } else if weight > 0 {
        "low"
    } else {
        "none"
    }
}
fn risks(findings: &[Value]) -> Value {
    let risk: Vec<_> = findings
        .iter()
        .filter(|f| f["attribution"] == "user_wallet" && small(&f["impact"]) < 0)
        .collect();
    let weight = risk.iter().map(|f| -small(&f["impact"])).sum();
    json!({"risk_weight":weight,"risk_count":risk.len(),"unknown_count":findings.iter().filter(|f| f["evidence_level"] == "unavailable").count(),"risk_level":risk_level(weight)})
}
fn merge(v: &mut Value, extra: Value) {
    v.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
}
fn top(findings: &[Value], n: usize) -> Value {
    json!(aggregate(findings).into_iter().take(n).collect::<Vec<_>>())
}
fn score(findings: &[Value]) -> i64 {
    (70 + findings.iter().map(|f| small(&f["impact"])).sum::<i64>()).clamp(0, 100)
}
fn active(row: &Value) -> bool {
    stripped(&row["spent_at"]).is_none()
}
fn address_facts(rows: &[Value]) -> Result<(Vec<Value>, Value)> {
    for row in rows.iter().filter(|r| active(r) && truth(&r["amount"])) {
        if integer(&row["amount"]).is_none() {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "invalid literal for int() with base 10: {}",
                    kassiber_pytext::repr_str(&string(&row["amount"]))
                ),
            ));
        }
    }
    let mut addresses: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    for row in rows {
        let a = stripped(&row["address"]);
        let k = kind(a.as_deref(), hex(&row["script_pubkey"]).as_deref());
        if k != "unknown" {
            *types.entry(k.into()).or_default() += 1;
        }
        if let Some(a) = a {
            addresses.entry(a).or_default().push(row);
        }
    }
    let reused: Vec<_> = addresses
        .values()
        .filter(|rows| {
            rows.iter()
                .map(|r| format!("{}:{}", string(&r["txid"]), string(&r["vout"])))
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        })
        .collect();
    let active_count = rows.iter().filter(|r| active(r)).count();
    let dust = rows
        .iter()
        .filter(|r| {
            active(r) && floor_sats(integer(&r["amount"]).unwrap_or_default()) < 1000.into()
        })
        .count();
    let mut f = Vec::new();
    if !reused.is_empty() {
        f.push(finding(
            "address_reuse",
            "critical",
            -(12 + reused.len() as i64 * 6).min(30),
            "wallet",
            reused.len(),
            json!({"outputs":reused.iter().map(|r| r.len()).sum::<usize>()}),
        ));
    }
    if dust > 0 {
        f.push(finding(
            "dust_utxo_exposure",
            "medium",
            -(4 + dust as i64 * 2).min(12),
            "wallet",
            dust,
            json!({"active_dust_utxos":dust}),
        ));
    }
    if active_count >= 20 {
        f.push(finding(
            "large_utxo_set",
            if active_count >= 50 { "high" } else { "medium" },
            if active_count >= 50 { -15 } else { -8 },
            "wallet",
            active_count,
            json!({"active_utxos":active_count}),
        ));
    }
    let legacy = types.get("p2pkh").copied().unwrap_or(0) + types.get("p2sh").copied().unwrap_or(0);
    if legacy > 0 {
        f.push(finding(
            "legacy_address_type",
            "medium",
            -(legacy as i64 * 2).min(10),
            "wallet",
            legacy,
            json!({"legacy_outputs":legacy}),
        ));
    }
    let tr = types.get("p2tr").copied().unwrap_or(0);
    if tr > 0 && tr >= (types.values().sum::<usize>() / 2).max(1) {
        f.push(finding(
            "taproot_usage",
            "positive",
            5,
            "wallet",
            tr,
            json!({"taproot_outputs":tr}),
        ));
    }
    let high = rows
        .iter()
        .filter(|r| integer(&r["anonymity_score"]).is_some_and(|n| n >= 20.into()))
        .count();
    if high > 0 {
        f.push(finding(
            "coin_anonymity_evidence",
            "positive",
            (4 + high as i64).min(10),
            "wallet",
            high,
            json!({"outputs":high}),
        ));
    }
    let summary = json!({"known_address_count":addresses.len(),"reused_address_count":addresses.values().filter(|r| r.len()>1).count(),"active_utxo_count":active_count,"dust_utxo_count":dust,"script_type_counts":types});
    Ok((f, summary))
}

fn query(conn: &Connection, sql: &str, args: &[String]) -> Result<Vec<Value>> {
    sql::rows(conn, sql, params_from_iter(args))?
        .iter()
        .map(json)
        .collect()
}
fn load_transactions(
    conn: &Connection,
    profile: &str,
    wallet: Option<&str>,
    reference: Option<&str>,
    excluded: bool,
) -> Result<Vec<Value>> {
    let mut filters = vec!["t.profile_id = ?"];
    let mut args = vec![profile.to_owned()];
    if !excluded {
        filters.push("t.excluded = 0");
    }
    if let Some(w) = wallet {
        filters.push("t.wallet_id = ?");
        args.push(w.into());
    }
    if let Some(r) = reference {
        filters.push("(t.id = ? OR lower(t.external_id) = lower(?))");
        args.extend([r.into(), r.into()]);
    }
    query(conn,&format!("SELECT t.*,w.label AS wallet_label,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE {} ORDER BY t.occurred_at DESC,t.created_at DESC,t.id DESC",filters.join(" AND ")),&args)
}
fn validation(message: &str, details: Value) -> AppError {
    AppError::new("validation", message).with_details(py(&details))
}

pub fn snapshot(conn: &Connection, args: &Py) -> Result<Py> {
    let args = match args {
        Py::Null => json!({}),
        Py::Dict(_) => {
            let mut normalized = args.clone();
            for key in ["wallet", "transaction"] {
                if let Some(v) =
                    kassiber_envelope::get(args, key).filter(|v| !matches!(v, Py::Null))
                {
                    kassiber_envelope::set(&mut normalized, key, kassiber_envelope::s(py_str(v)));
                }
            }
            json(&normalized)?
        }
        _ => {
            return Err(validation(
                "ui.privacy_hygiene.snapshot args must be an object",
                json!({"type":kassiber_envelope::type_name(args)}),
            ))
        }
    };
    let unknown: Vec<_> = args
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| !["wallet", "transaction", "limit"].contains(&k.as_str()))
        .collect();
    if !unknown.is_empty() {
        return Err(validation(
            "ui.privacy_hygiene.snapshot received unsupported fields",
            json!({"unknown":unknown}),
        ));
    }
    snapshot_inner(conn, &args).map(|v| py(&v))
}
fn snapshot_inner(conn: &Connection, args: &Value) -> Result<Value> {
    let context = json(&kassiber_setup::context::snapshot(conn)?)?;
    let Some(profile) = stripped(&context["profile_id"]) else {
        let mut meta = meta();
        meta.as_object_mut().unwrap().remove("phase2_deferred");
        meta["context"] = json!({"workspace":stripped(&context["workspace_label"]),"profile":stripped(&context["profile_label"])});
        let mut summary = risks(&[]);
        merge(
            &mut summary,
            json!({"state":"no_active_profile","wallet_count":0,"transaction_count":0,"finding_counts":counts(&[]),"top_findings":[]}),
        );
        return Ok(
            json!({"profile":null,"scope":{"wallet":null,"transaction":null},"summary":summary,"coverage":coverage(&[],&[],&[]),"wallets":[],"transactions":[],"findings":[],"meta":meta}),
        );
    };
    let wallet = if let Some(w) = stripped(&args["wallet"]) {
        Some(query(conn,"SELECT id,label,kind FROM wallets WHERE profile_id=? AND (id=? OR label=?) ORDER BY CASE WHEN id=? THEN 0 ELSE 1 END,label ASC LIMIT 1",&[profile.clone(),w.clone(),w.clone(),w.clone()])?.into_iter().next()
            .ok_or_else(|| AppError::new("not_found","Wallet not found for privacy-hygiene scan").with_details(py(&json!({"wallet":w}))))?)
    } else {
        None
    };
    let wallet_id = wallet.as_ref().map(|w| string(&w["id"]));
    let reference = stripped(&args["transaction"]);
    let limit = if args["limit"].is_null() || args["limit"] == "" {
        50
    } else {
        let n = if args["limit"].is_boolean() {
            Some(BigInt::from(args["limit"] == true))
        } else {
            integer(&args["limit"])
        }
        .ok_or_else(|| {
            validation(
                "Privacy-hygiene transaction limit must be an integer",
                json!({"limit":args["limit"]}),
            )
        })?;
        if n < 1.into() {
            return Err(validation(
                "Privacy-hygiene transaction limit must be positive",
                json!({"limit":args["limit"]}),
            ));
        }
        usize::try_from(n.min(200.into())).unwrap()
    };
    let mut params = vec![profile.clone()];
    if let Some(w) = &wallet_id {
        params.push(w.clone());
    }
    let wallets = query(
        conn,
        &format!(
            "SELECT id,label,kind FROM wallets WHERE profile_id=? {} ORDER BY label ASC,id ASC",
            if wallet_id.is_some() { "AND id=?" } else { "" }
        ),
        &params,
    )?;
    let inventory = query(conn,&format!("SELECT u.*,w.label AS wallet_label,w.kind AS wallet_kind FROM wallet_utxos u JOIN wallets w ON w.id=u.wallet_id WHERE u.profile_id=? {} ORDER BY u.wallet_id ASC,u.txid ASC,u.vout ASC",if wallet_id.is_some() {"AND u.wallet_id=?"} else {""}),&params)?;
    let transactions = load_transactions(
        conn,
        &profile,
        wallet_id.as_deref(),
        reference.as_deref(),
        false,
    )?;
    if let Some(r) = &reference {
        if transactions.is_empty() {
            return Err(AppError::new(
                "not_found",
                "Transaction not found for privacy-hygiene scan",
            )
            .with_details(py(&json!({"transaction":r}))));
        }
    }
    let mut collaborations = BTreeMap::new();
    for row in load_transactions(conn, &profile, None, None, true)? {
        if let (Some(id), Some(c)) = (identity(&row), collaboration(&row)) {
            collaborations.insert(id, c);
        }
    }
    let index = inventory_index(&inventory);
    let mut txs = Vec::new();
    for row in transactions {
        let p = parse(&row, &index);
        let c = identity(&row)
            .and_then(|id| collaborations.get(&id))
            .cloned()
            .or_else(|| collaboration(&row));
        let f = transaction_findings(&row, &p, c.as_ref());
        let mut tx = row.clone();
        merge(
            &mut tx,
            json!({"score":if p.support == "none" {Value::Null} else {json!(score(&f))},"state":if p.support == "none" {"not_analysable"} else {p.support},"support":{"level":p.support,"reason":p.reason,"input_count":p.inputs.len(),"output_count":p.outputs.len(),"known_input_values":p.inputs.iter().filter(|v| v.value.is_some()).count(),"known_output_values":p.outputs.iter().filter(|v| v.value.is_some()).count()},"findings":f,"finding_counts":counts(&f)}),
        );
        txs.push(tx);
    }
    let mut wallet_results = Vec::new();
    let mut all = Vec::new();
    let mut wallet_states = Vec::new();
    for wallet in &wallets {
        let inv: Vec<_> = inventory
            .iter()
            .filter(|r| r["wallet_id"] == wallet["id"])
            .cloned()
            .collect();
        let tx: Vec<_> = txs
            .iter()
            .filter(|r| r["wallet_id"] == wallet["id"])
            .collect();
        let (mut f, address) = address_facts(&inv)?;
        let scores: Vec<_> = tx
            .iter()
            .filter(|r| !r["score"].is_null())
            .map(|r| small(&r["score"]))
            .collect();
        let state = if !scores.is_empty() {
            if tx.iter().any(|r| r["state"] != "full") {
                "partial"
            } else {
                "full"
            }
        } else if !inv.is_empty() {
            "address_only"
        } else {
            "not_enough_data"
        };
        let count = tx.iter().filter(|r| r["state"] == "not_analysable").count();
        if count > 0 {
            f.push(finding(
                "transaction_coverage_gap",
                "info",
                0,
                "wallet",
                count,
                json!({"transactions":count}),
            ));
        }
        sort_findings(&mut f);
        all.extend(f.clone());
        let mut risk = f;
        risk.extend(tx.iter().flat_map(|r| list(&r["findings"]).iter().cloned()));
        sort_findings(&mut risk);
        let mut result = wallet.clone();
        merge(
            &mut result,
            json!({"state":state,"transaction_count":tx.len(),"scored_transaction_count":scores.len(),"inventory_output_count":inv.len(),"active_utxo_count":inv.iter().filter(|r| active(r)).count(),"address":address,"finding_counts":counts(&risk),"top_findings":top(&risk,5)}),
        );
        merge(&mut result, risks(&risk));
        wallet_results.push(result);
        wallet_states.push(state);
    }
    all.extend(
        txs.iter()
            .flat_map(|r| list(&r["findings"]).iter().cloned()),
    );
    let mut summary = risks(&all);
    let state = if wallets.is_empty() {
        "no_wallets"
    } else if wallet_states.iter().all(|s| *s == "not_enough_data") {
        "not_enough_data"
    } else if wallet_states.iter().any(|s| *s != "full") {
        "partial"
    } else {
        "full"
    };
    merge(
        &mut summary,
        json!({"state":state,"wallet_count":wallets.len(),"transaction_count":txs.len(),"finding_counts":counts(&all),"top_findings":top(&all,5)}),
    );
    let coverage = coverage(&wallets, &inventory, &txs);
    txs.sort_by_key(|r| {
        (
            r["score"].is_null(),
            if r["score"].is_null() {
                101
            } else {
                small(&r["score"])
            },
            -list(&r["findings"])
                .iter()
                .map(|f| severity(&f["severity"]))
                .max()
                .unwrap_or(0),
            string(&r["occurred_at"]),
        )
    });
    let returned: Vec<_> = txs
        .iter()
        .take(limit)
        .map(|tx| {
            let mut result = json!({});
            for key in [
                "id",
                "wallet_id",
                "wallet_label",
                "occurred_at",
                "direction",
                "asset",
                "state",
                "support",
                "finding_counts",
            ] {
                result[key] = tx[key].clone();
            }
            if !truth(&result["wallet_label"]) {
                result["wallet_label"] = json!("");
            }
            merge(&mut result, risks(list(&tx["findings"])));
            result["top_findings"] = top(list(&tx["findings"]), 8);
            result
        })
        .collect();
    let profile_row = query(
        conn,
        "SELECT id,label FROM profiles WHERE id=?",
        std::slice::from_ref(&profile),
    )?
    .into_iter()
    .next()
    .unwrap_or_else(|| json!({"id":profile,"label":context["profile_label"]}));
    Ok(
        json!({"profile":profile_row,"scope":{"wallet":wallet.map(|w| json!({"id":w["id"],"label":w["label"]})),"transaction":reference},"summary":summary,"coverage":coverage,"wallets":wallet_results,"transactions":returned,"findings":top(&all,12),"meta":meta()}),
    )
}
fn coverage(wallets: &[Value], inventory: &[Value], txs: &[Value]) -> Value {
    json!({"wallet_count":wallets.len(),"wallets_with_inventory":inventory.iter().map(|v|string(&v["wallet_id"])).collect::<BTreeSet<_>>().len(),"inventory_outputs":inventory.len(),"active_utxos":inventory.iter().filter(|v|active(v)).count(),"transaction_total":txs.len(),"transaction_full":txs.iter().filter(|v|v["state"]=="full").count(),"transaction_partial":txs.iter().filter(|v|v["state"]=="partial").count(),"transaction_not_analysable":txs.iter().filter(|v|v["state"]=="not_analysable").count(),"transaction_scored":txs.iter().filter(|v|!v["score"].is_null()).count()})
}
fn meta() -> Value {
    json!({"engine":"privacy_hygiene_phase1","local_only":true,"egress":"none","scope":"single_transaction_and_address_level","phase2_deferred":["cross_wallet_cluster_reconstruction","peel_chain_detection","boltzmann_entropy","coin_selection_fingerprint","psbt_prebroadcast_check"]})
}

#[cfg(feature = "test-harness")]
pub fn fixture_probes(conn: &Connection) -> Result<Value> {
    let inventory = query(conn,"SELECT u.*,w.label AS wallet_label,w.kind AS wallet_kind FROM wallet_utxos u JOIN wallets w ON w.id=u.wallet_id WHERE u.profile_id='p' ORDER BY u.wallet_id,u.txid,u.vout",&[])?;
    let index = inventory_index(&inventory);
    let rows = load_transactions(conn, "p", None, None, true)?;
    let mut context = BTreeMap::new();
    for row in &rows {
        if let (Some(id), Some(c)) = (identity(row), collaboration(row)) {
            context.insert(id, c);
        }
    }
    Ok(json!(rows.iter().map(|row| {
        let p = parse(row,&index);
        let c = identity(row).and_then(|id| context.get(&id)).cloned().or_else(|| collaboration(row));
        let f = transaction_findings(row,&p,c.as_ref());
        json!({"id":row["id"],"score":if p.support == "none" {Value::Null} else {json!(score(&f))},"state":if p.support == "none" {"not_analysable"} else {p.support},"findings":f})
    }).collect::<Vec<_>>()))
}

#[cfg(test)]
mod coinjoin_shape_tests {
    use super::*;

    #[test]
    fn coinjoin_shapes_match_python_for_collaboration_and_hygiene() {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../../wallets/tests/data/coinjoin_shapes.json"
        ))
        .unwrap();
        assert!(cases.len() > 60);
        for case in &cases {
            let name = case["name"].as_str().unwrap();
            let raw = serde_json::to_string(&case["raw"]).unwrap();
            let row = json!({"chain":"bitcoin","network":"main","asset":"BTC","raw_json":raw,
                "external_id":null,"privacy_boundary":null,"review_status":null,"fee":null});
            assert_eq!(
                collaboration(&row).unwrap_or(Value::Null),
                case["collaboration"],
                "{name}"
            );
            let p = parse(&row, &Inventory::default());
            let signal = if p.support == "none" {
                None
            } else {
                parsed_coinjoin(&row, &object(&row["raw_json"]), &p)
            };
            assert_eq!(signal.unwrap_or(Value::Null), case["hygiene"], "{name}");
        }
    }
}
