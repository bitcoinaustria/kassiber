//! Finite, local source evidence used by pre-build overlap repairs.
use crate::{array, pyjson, string, truth, Refusal};
use kassiber_store::rusqlite::{types::ValueRef, Connection, Params};
use kassiber_wallets::{config, descriptors};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(super) fn rows(conn: &Connection, sql: &str, args: impl Params) -> Result<Vec<Value>, Refusal> {
    let mut stmt = conn.prepare(sql)?;
    let names: Vec<_> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut cursor = stmt.query(args)?;
    let mut output = Vec::new();
    while let Some(row) = cursor.next()? {
        let mut fields = serde_json::Map::new();
        for (i, name) in names.iter().enumerate() {
            fields.insert(name.clone(), value(row.get_ref(i)?)?);
        }
        output.push(Value::Object(fields));
    }
    Ok(output)
}

/// One stored scalar; an infinite REAL refuses as `int()` would.
pub(super) fn value(v: ValueRef<'_>) -> Result<Value, Refusal> {
    Ok(match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => json!(n),
        ValueRef::Real(n) => {
            if !n.is_finite() {
                return Err(Refusal::new(
                    "OverflowError",
                    "cannot convert float infinity to integer",
                ));
            }
            json!(n)
        }
        ValueRef::Text(s) => {
            json!(std::str::from_utf8(s).map_err(|_| Refusal::new("ValueError", "invalid UTF-8"))?)
        }
        ValueRef::Blob(value) => {
            kassiber_custody::codec::to_json(&kassiber_pyjson::Value::Bytes(value.to_vec()))
        }
    })
}

pub(super) fn text(v: &Value) -> String {
    if truth(v) {
        string(v)
    } else {
        String::new()
    }
}
pub(super) fn lower(v: &str) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(v))
}
pub(super) fn mapping(v: &Value) -> Value {
    let v = match v {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
        _ => v.clone(),
    };
    if v.is_object() {
        v
    } else {
        json!({})
    }
}
pub(super) fn script(v: &Value) -> Option<String> {
    let s = lower(&text(v));
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = s.bytes();
    while let Some(byte) = bytes.next() {
        if b" \t\n\r\x0b\x0c".contains(&byte) {
            continue;
        }
        if !byte.is_ascii_hexdigit() || !bytes.next().is_some_and(|b| b.is_ascii_hexdigit()) {
            return None;
        }
    }
    Some(s)
}
pub(super) fn addresses(raw: &Value) -> Result<Vec<String>, Refusal> {
    use kassiber_pyjson::Value as Py;
    let config = match raw {
        Value::String(s) => kassiber_pytext::loads(s).unwrap_or(Py::Null),
        _ => kassiber_custody::codec::from_json(raw).unwrap_or(Py::Null),
    };
    let address = kassiber_envelope::get(&config, "addresses").unwrap_or(&Py::Null);
    let values = match address {
        Py::Null => vec![],
        Py::Str(s) => s.as_str().unwrap_or("").split(',').map(Py::from).collect(),
        Py::List(v) => v.clone(),
        Py::Dict(v) => v
            .iter()
            .map(|(k, _)| match k {
                kassiber_pyjson::Key::Str(s) => Py::Str(s.clone()),
                _ => unreachable!("JSON keys are strings"),
            })
            .collect(),
        Py::Bool(_) => return Err(Refusal::new("TypeError", "'bool' object is not iterable")),
        Py::Int(_) => return Err(Refusal::new("TypeError", "'int' object is not iterable")),
        _ => return Err(Refusal::new("TypeError", "'float' object is not iterable")),
    };
    let mut seen = BTreeSet::new();
    Ok(values
        .iter()
        .filter_map(|v| {
            let v = kassiber_envelope::py_str(v);
            let address = kassiber_pytext::strip(&v);
            if address.is_empty() {
                return None;
            }
            let key = config::bitcoin_address_script(address)
                .map(|s| format!("script:{s}"))
                .unwrap_or_else(|| format!("text:{address}"));
            seen.insert(key).then(|| address.to_string())
        })
        .collect())
}
fn deprecated(c: &Value) -> Result<bool, Refusal> {
    if c["deprecated"].is_null() {
        return Ok(false);
    }
    match lower(&string(&c["deprecated"])).as_str() {
        "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        _ => {
            let mut e = Refusal::new(
                "AppError",
                format!("Invalid boolean value: {}", string(&c["deprecated"])),
            );
            e.code = Some("app_error".into());
            Err(e)
        }
    }
}
fn scope(chain: &Value, network: &Value) -> (String, String) {
    if chain.is_null() || chain == "" || network.is_null() || network == "" {
        return (String::new(), String::new());
    }
    let chain = text(chain);
    let network = text(network);
    if let Ok(c) = descriptors::normalize_chain(&chain) {
        if let Ok(n) = descriptors::normalize_network(&c, &network) {
            return (c, n);
        }
    }
    (lower(&chain), lower(&network))
}

#[derive(Clone)]
pub(super) struct Source {
    pub wallet: String,
    pub label: String,
    pub kind: String,
    pub chain: String,
    pub network: String,
    pub script: String,
    pub source: &'static str,
    pub deprecated: bool,
    pub count: i64,
}

fn integer(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|v| v.is_finite())
                .map(|v| v.trunc() as i64)
        }),
        Value::String(s) => kassiber_pytext::parse_int(s).ok().map(|n| {
            i64::try_from(n.clone()).unwrap_or(if n < 0.into() { i64::MIN } else { i64::MAX })
        }),
        _ => None,
    }
}
type Key = (String, String, String);
pub(super) struct Index {
    pub sources: Vec<Source>,
    pub groups: Vec<(Key, Vec<Source>)>,
}

pub(super) fn index(conn: &Connection, profile: &str) -> Result<Index, Refusal> {
    let counts: HashMap<String, i64> = rows(conn, "SELECT wallet_id,COUNT(*) AS count FROM transactions WHERE profile_id=? AND excluded=0 GROUP BY wallet_id", [profile])?
        .iter().map(|r| (text(&r["wallet_id"]), r["count"].as_i64().unwrap_or(0))).collect();
    let wallets = rows(conn, "SELECT id,label,kind,config_json FROM wallets WHERE profile_id=? ORDER BY label ASC,id ASC", [profile])?;
    let make = |w: &Value,
                c: &Value,
                chain: String,
                network: String,
                script: String,
                source|
     -> Result<Source, Refusal> {
        let wallet = text(&w["id"]);
        Ok(Source {
            label: if truth(&w["label"]) {
                text(&w["label"])
            } else {
                wallet.clone()
            },
            kind: text(&w["kind"]),
            chain,
            network,
            script,
            source,
            deprecated: deprecated(c)?,
            count: *counts.get(&wallet).unwrap_or(&0),
            wallet,
        })
    };
    let mut sources = Vec::new();
    for w in &wallets {
        let c = mapping(&w["config_json"]);
        let (chain, network) = scope(&c["chain"], &c["network"]);
        if chain.is_empty() || network.is_empty() {
            continue;
        }
        for address in addresses(&w["config_json"])? {
            if let Some(script) = config::bitcoin_address_script(&address) {
                let mut source = make(
                    w,
                    &c,
                    chain.clone(),
                    network.clone(),
                    script,
                    "address_list",
                )?;
                if source.kind.is_empty() {
                    source.kind = "address".into();
                }
                sources.push(source);
            }
        }
    }
    let inventory = rows(conn, "SELECT DISTINCT wallet_id,chain,network,address,script_pubkey,branch_label,branch_index,address_index,raw_json FROM wallet_utxos WHERE profile_id=?", [profile])?;
    for row in &inventory {
        let s = script(&row["script_pubkey"])
            .or_else(|| script(&mapping(&row["raw_json"])["script_pubkey"]))
            .or_else(|| config::bitcoin_address_script(&text(&row["address"])));
        let Some(s) = s else {
            continue;
        };
        let missing = json!({"id":row["wallet_id"]});
        let w = wallets
            .iter()
            .find(|w| w["id"] == row["wallet_id"])
            .unwrap_or(&missing);
        let (chain, network) = scope(&row["chain"], &row["network"]);
        sources.push(make(
            w,
            &mapping(&w["config_json"]),
            chain,
            network,
            s,
            "inventory",
        )?);
    }
    let mut maxes: HashMap<(String, u32), i64> = HashMap::new();
    for r in rows(conn, "SELECT wallet_id,branch_index,MAX(address_index) AS max_index FROM wallet_utxos WHERE profile_id=? AND branch_index IS NOT NULL AND address_index IS NOT NULL GROUP BY wallet_id,branch_index", [profile])? {
        if let (Some(b), Some(n)) = (integer(&r["branch_index"]).and_then(|v| u32::try_from(v).ok()), integer(&r["max_index"])) { maxes.insert((text(&r["wallet_id"]), b), n.max(-1)); }
    }
    for r in rows(conn, "SELECT source_key,checkpoint_json FROM freshness_source_states WHERE profile_id=? AND source_key LIKE 'onchain_wallet:%'", [profile])? {
        let c = mapping(&r["checkpoint_json"]);
        if let Some(highest) = c["highest_used"].as_object() {
            for (b, n) in highest {
                if let (Some(b), Some(n)) = (integer(&json!(b)).and_then(|v| u32::try_from(v).ok()), integer(n)) {
                    let key = (text(&r["source_key"])["onchain_wallet:".len()..].to_string(), b);
                    maxes.entry(key).and_modify(|v| *v = (*v).max(n).max(-1)).or_insert(n.max(-1));
                }
            }
        }
    }
    for w in &wallets {
        let c = mapping(&w["config_json"]);
        let material = !text(&c["descriptor"]).trim().is_empty()
            || (!text(&c["xpub"]).trim().is_empty() && truth(&c["script_types"]));
        if !material {
            continue;
        }
        let Ok(Some(plan)) = descriptors::load(&c) else {
            continue;
        };
        let wallet = text(&w["id"]);
        for b in &plan.branches {
            let end = maxes
                .get(&(wallet.clone(), b.index))
                .map_or(i64::from(plan.gap_limit), |n| {
                    n.saturating_add(i64::from(plan.gap_limit) + 1)
                })
                .clamp(0, 20_000) as u32;
            let Ok(targets) = plan.derive(Some(b.index), 0, end) else {
                continue;
            };
            for t in targets {
                sources.push(make(
                    w,
                    &c,
                    plan.chain.clone(),
                    plan.network.clone(),
                    lower(&text(&t["script_pubkey"])),
                    "descriptor_config",
                )?);
            }
        }
    }
    sources.retain(|s| !s.deprecated || s.count > 0);
    let mut groups: Vec<(Key, Vec<Source>)> = Vec::new();
    let mut positions = HashMap::new();
    for source in &sources {
        if source.chain.is_empty() || source.network.is_empty() {
            continue;
        }
        let key = (
            source.chain.clone(),
            source.network.clone(),
            source.script.clone(),
        );
        let i = *positions.entry(key.clone()).or_insert_with(|| {
            groups.push((key, Vec::new()));
            groups.len() - 1
        });
        groups[i].1.push(source.clone());
    }
    Ok(Index { sources, groups })
}

fn priority(s: &str) -> i64 {
    match s {
        "inventory" | "descriptor_config" => 0,
        "address_list" => 1,
        _ => 2,
    }
}
pub(super) fn canonical(kind: &str) -> bool {
    matches!(kind, "descriptor" | "xpub" | "silent-payment")
}
pub(super) fn payload(key: &Key, sources: &[Source]) -> Value {
    let mut wallets = BTreeMap::<(&str, &str), Value>::new();
    let mut evidence = BTreeSet::new();
    let mut counts = BTreeMap::<(&str, &str), usize>::new();
    for s in sources {
        evidence.insert(s.source);
        let w = wallets.entry((&s.label, &s.wallet)).or_insert_with(|| json!({"id":s.wallet,"label":s.label,"kind":s.kind,"deprecated":s.deprecated,"active_transaction_count":s.count,"sources":[],"_source_priority":99}));
        if !array(&w["sources"]).contains(&json!(s.source)) {
            w["sources"].as_array_mut().unwrap().push(json!(s.source));
        }
        w["_source_priority"] = json!(w["_source_priority"]
            .as_i64()
            .unwrap()
            .min(priority(s.source)));
        if s.source == "descriptor_config" {
            *counts.entry((&s.label, &s.wallet)).or_default() += 1;
        }
    }
    let selected = wallets
        .values()
        .min_by_key(|w| {
            (
                w["deprecated"].as_bool().unwrap(),
                !canonical(&text(&w["kind"])),
                w["_source_priority"].as_i64().unwrap(),
                w["active_transaction_count"] == 0,
                text(&w["label"]),
                text(&w["id"]),
            )
        })
        .unwrap()
        .clone();
    let mut repairs = Vec::new();
    for w in wallets.values_mut() {
        w.as_object_mut().unwrap().remove("_source_priority");
        let n = sources
            .iter()
            .filter(|s| s.wallet == text(&w["id"]) && s.source == "address_list")
            .map(|s| &s.script)
            .collect::<BTreeSet<_>>()
            .len();
        if w["id"] != selected["id"] && n > 0 {
            repairs.push(json!({"wallet_id":w["id"],"wallet":w["label"],"overlapping_address_list_target_count":n,"action":"remove_overlapping_address_list_targets","clear_output_inventory":true,"reset_onchain_refresh_checkpoint":true,"deprecate_if_empty_after_trim":true,"requires_confirmation":true}));
        }
    }
    json!({"chain":key.0,"network":key.1,"wallets":wallets.into_values().collect::<Vec<_>>(),"recommended_canonical_wallet_id":selected["id"],"recommended_canonical_wallet":selected["label"],"recommendation_reason":"Prefer active, non-deprecated descriptor/xpub sources as canonical when the overlap is otherwise equivalent; this is a recommendation, not an automatic irreversible repair.","evidence":evidence,"address_list_repair_preview":repairs,"checked":{"scope":"finite_scripts_only","descriptor_global_overlap_proven":false,"note":"Only resolved sync targets, address-list scripts, stored descriptor targets within the checked horizon, and existing wallet_utxos evidence were compared. Address-list entries beyond a descriptor's checked horizon are not proven overlapping.","target_counts":counts.into_iter().map(|((label,id),n)|json!({"wallet_id":id,"wallet":label,"target_count":n})).collect::<Vec<_>>()}})
}

impl Index {
    pub fn overlaps(&self) -> Vec<Value> {
        let mut out: Vec<_> = self
            .groups
            .iter()
            .filter(|(_, g)| g.iter().map(|s| &s.wallet).collect::<BTreeSet<_>>().len() > 1)
            .map(|(k, g)| payload(k, g))
            .collect();
        out.sort_by_key(|v| {
            (
                text(&v["chain"]),
                text(&v["network"]),
                text(&v["wallets"][0]["label"]),
            )
        });
        out
    }
    pub fn projection(&self) -> Value {
        let overlaps = self.overlaps();
        json!({"overlap_count":overlaps.len(),"overlaps":overlaps,"checked":{"scope":"finite_scripts_only","source_count":self.sources.len(),"wallet_count":self.sources.iter().map(|s| &s.wallet).collect::<BTreeSet<_>>().len(),"descriptor_global_overlap_proven":false}})
    }
}

pub(super) fn stored_config(config: &Value) -> Result<String, Refusal> {
    pyjson(config, false)
}
