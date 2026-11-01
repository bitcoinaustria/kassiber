use super::{field, list, Text};
use elements_miniscript::elements::hex::ToHex;
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_pyjson::{dumps, BigInt, Options, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn error(message: &str) -> AppError {
    AppError::new("validation", message)
}
fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    field(v, k).as_str().unwrap_or("")
}
fn number(n: usize) -> Value {
    Value::Int(BigInt::from(n as u64))
}
fn domain(chain: &Value, network: &Value) -> Result<(String, String), AppError> {
    let c = chain.as_str().unwrap_or("");
    let Some(n) = network.as_str().filter(|n| !n.is_empty()) else {
        return Err(error(
            "An explicit Bitcoin or Liquid chain and network are required",
        ));
    };
    if !["bitcoin", "liquid"].contains(&c) {
        return Err(error(
            "An explicit Bitcoin or Liquid chain and network are required",
        ));
    }
    crate::descriptors::normalize_network(c, n)
        .map(|n| (c.into(), n))
        .map_err(|_| error("Unsupported attribution network"))
}

pub fn canonical_subject(
    chain: &Value,
    network: &Value,
    subject: &Value,
) -> Result<Value, AppError> {
    let (chain, network) = domain(chain, network)?;
    let value = subject
        .as_str()
        .filter(|v| {
            !kassiber_pytext::strip(v).is_empty()
                && v.chars().count() <= 20064
                && !v.chars().any(|c| (c as u32) < 32)
        })
        .ok_or_else(|| error("subject must be nonempty text up to 20064 characters"))?;
    let mut value = kassiber_pytext::strip(value);
    let parts = value.splitn(3, ':').collect::<Vec<_>>();
    if parts.len() == 3 && ["bitcoin", "liquid"].contains(&parts[0]) {
        if domain(&s(parts[0]), &s(parts[1]))? != (chain.clone(), network.clone()) {
            return Err(error(
                "Attribution subject belongs to a different chain or network",
            ));
        }
        value = parts[2];
    }
    let physical = value
        .strip_prefix("tx:")
        .or_else(|| value.strip_prefix("out:"))
        .unwrap_or(value);
    let txid = |v: &str| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit());
    let key = if txid(physical) {
        if value.starts_with("out:") {
            return Err(error("An output subject requires an output index"));
        }
        Some(format!("tx:{}", physical.to_lowercase()))
    } else if let Some((tx, index)) = physical.split_once(':') {
        if !value.starts_with("tx:")
            && txid(tx)
            && index.len() <= 10
            && (index == "0" || !index.starts_with('0'))
            && !index.is_empty()
            && index.bytes().all(|b| b.is_ascii_digit())
            && index.parse::<u32>().is_ok()
        {
            Some(format!("out:{}:{index}", tx.to_lowercase()))
        } else {
            None
        }
    } else {
        None
    };
    let key = key.or_else(|| {
        value
            .strip_prefix("script:")
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 20000
                    && v.len() % 2 == 0
                    && v.bytes().all(|b| b.is_ascii_hexdigit())
            })
            .map(|_| value.to_lowercase())
    });
    let key = if let Some(key) = key {
        key
    } else {
        let invalid = || {
            error("Attribution subject must be a valid address, transaction, output or script in the selected domain")
        };
        let script = if chain == "bitcoin" {
            let address = value
                .parse::<bitcoin::Address<_>>()
                .map_err(|_| invalid())?;
            let bitcoin_network = match network.as_str() {
                "main" => bitcoin::Network::Bitcoin,
                "regtest" => bitcoin::Network::Regtest,
                "signet" => bitcoin::Network::Signet,
                _ => bitcoin::Network::Testnet,
            };
            address
                .require_network(bitcoin_network)
                .map_err(|_| invalid())?
                .script_pubkey()
                .to_hex_string()
        } else {
            let address = value
                .parse::<elements_miniscript::elements::Address>()
                .map_err(|_| invalid())?;
            let params = match network.as_str() {
                "liquidv1" => &elements_miniscript::elements::AddressParams::LIQUID,
                "liquidtestnet" => &elements_miniscript::elements::AddressParams::LIQUID_TESTNET,
                _ => &elements_miniscript::elements::AddressParams::ELEMENTS,
            };
            if address.params != params || address.script_pubkey().is_p2pkh() {
                return Err(invalid());
            }
            address.script_pubkey().to_hex()
        };
        format!("script:{script}")
    };
    Ok(Value::List(vec![s(chain), s(network), s(key)]))
}

pub(super) fn matched(pack: &Value, row: &Value) -> Result<Value, AppError> {
    let manifest = field(pack, "manifest");
    let source = kassiber_pytext::loads(text(row, "source_record_json"))
        .map_err(|_| error("Invalid captured attribution source record"))?;
    if !matches!(source, Value::Dict(_)) {
        return Err(error("Invalid captured attribution source record"));
    }
    let record = kassiber_envelope::py_str(field(row, "record_number"));
    Ok(dict([
        ("id", s(format!("dataset:{}:{record}", text(pack, "id")))),
        ("revision", number(1)),
        ("dataset_id", field(pack, "id").clone()),
        ("dataset_status", field(pack, "status").clone()),
        ("chain", field(pack, "chain").clone()),
        ("network", field(pack, "network").clone()),
        ("subject", field(row, "subject").clone()),
        ("label", field(row, "label").clone()),
        ("category", field(row, "category").clone()),
        (
            "source",
            get(&source, "source")
                .unwrap_or(field(manifest, "source"))
                .clone(),
        ),
        ("confidence", s("imported")),
        ("cluster_defining", Value::Bool(false)),
        ("visibility", field(pack, "visibility").clone()),
        (
            "attribution_method",
            get(&source, "attribution_method")
                .unwrap_or(field(manifest, "attribution_method"))
                .clone(),
        ),
        ("source_record", source.clone()),
        ("valid_from", field(row, "valid_from").clone()),
        ("valid_until", field(row, "valid_until").clone()),
        ("dataset_version", field(pack, "version").clone()),
        ("content_sha256", field(pack, "content_sha256").clone()),
        ("license", field(manifest, "license").clone()),
        ("evidence_level", s("sourced_claim")),
        ("accounting_authority", Value::Bool(false)),
    ]))
}
fn valid_at(v: &Value, instant: &str) -> bool {
    (!truthy(field(v, "valid_from")) || text(v, "valid_from") <= instant)
        && (!truthy(field(v, "valid_until")) || text(v, "valid_until") > instant)
}
struct Partition {
    found: Vec<Value>,
    active: usize,
    inactive: usize,
    lookups: usize,
    truncated: bool,
}
impl Partition {
    fn new() -> Self {
        Self {
            found: vec![],
            active: 0,
            inactive: 0,
            lookups: 0,
            truncated: false,
        }
    }
    fn coverage(&self, limit: usize) -> Value {
        dict([
            ("match_count", number(self.found.len().min(limit))),
            ("active_dataset_count", number(self.active)),
            ("inactive_by_date_count", number(self.inactive)),
            ("lookup_queries", number(self.lookups)),
            ("match_limit", number(limit)),
            ("truncated", Value::Bool(self.truncated)),
        ])
    }
}

pub fn match_captured(
    packs: &Value,
    claims: &Value,
    subjects: &Value,
    observer: &str,
    limit: &Value,
    as_of: &Value,
) -> Result<Value, AppError> {
    let n = if let Value::Int(n) = limit {
        n.to_string().parse::<usize>().unwrap_or(0)
    } else {
        0
    };
    if !["public", "owner", "disclosed"].contains(&observer) || !(1..=20000).contains(&n) {
        return Err(error("Invalid dataset observer or match limit"));
    }
    let instant = if as_of == &Value::Null || as_of == &s("") {
        None
    } else {
        as_of.as_str().filter(|v| {
            !kassiber_pytext::strip(v).is_empty()
                && v.chars().count() <= 40
                && !v.chars().any(|c| (c as u32) < 32)
        })
    };
    let instant = instant
        .and_then(|v| crate::time::timestamp(v).ok())
        .ok_or_else(|| error("as_of must be an ISO date or timestamp"))?;
    let mut grouped: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for (i, subject) in list(subjects).iter().enumerate() {
        if i >= 100000 {
            return Err(AppError::new(
                "chain_analysis_dataset_limit",
                "Too many observed attribution subjects",
            ));
        }
        let parts = list(subject);
        if parts.len() != 3 {
            return Err(error(
                "Observed subjects must be chain, network, subject triples",
            ));
        }
        let key = canonical_subject(&parts[0], &parts[1], &parts[2])?;
        let parts = list(&key);
        grouped
            .entry((
                parts[0].as_str().unwrap().into(),
                parts[1].as_str().unwrap().into(),
            ))
            .or_default()
            .insert(parts[2].as_str().unwrap().into());
    }
    let mut packs = list(packs)
        .iter()
        .filter(|p| {
            text(p, "status") == "active"
                && (observer != "public" || text(p, "visibility") == "public")
        })
        .cloned()
        .collect::<Vec<_>>();
    packs.sort_by_key(|p| text(p, "id").to_owned());
    let encoded = dumps(
        &Value::List(packs.clone()),
        &Options::new()
            .sort_keys(true)
            .compact()
            .ensure_ascii(false)
            .allow_nan(false),
    )
    .map_err(|_| error("Invalid captured attribution manifest"))?;
    let state: String = Sha256::digest(encoded.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let names = if observer == "public" {
        vec!["public"]
    } else {
        vec!["public", "private"]
    };
    let mut partitions: BTreeMap<_, _> = names.iter().map(|v| (*v, Partition::new())).collect();
    for pack in &packs {
        let part = partitions
            .get_mut(text(pack, "visibility"))
            .ok_or_else(|| error("Invalid captured attribution visibility"))?;
        part.active += 1;
        if !valid_at(field(pack, "manifest"), &instant) {
            part.inactive += 1;
            continue;
        }
        if part.truncated {
            continue;
        }
        let keys = grouped
            .get(&(text(pack, "chain").into(), text(pack, "network").into()))
            .map(|v| v.iter().map(String::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        for chunk in keys.chunks(200) {
            part.lookups += 1;
            let mut rows = list(claims)
                .iter()
                .filter(|r| {
                    field(r, "dataset_id") == field(pack, "id")
                        && chunk.contains(&text(r, "subject"))
                        && valid_at(r, &instant)
                })
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| {
                text(a, "subject").cmp(text(b, "subject")).then_with(|| {
                    let a = if let Value::Int(n) = field(a, "record_number") {
                        n.clone()
                    } else {
                        BigInt::from(0)
                    };
                    let b = if let Value::Int(n) = field(b, "record_number") {
                        n.clone()
                    } else {
                        BigInt::from(0)
                    };
                    a.cmp(&b)
                })
            });
            for row in rows.into_iter().take(n + 1 - part.found.len()) {
                part.found.push(matched(pack, row)?);
            }
            if part.found.len() > n {
                part.truncated = true;
                break;
            }
        }
    }
    let found = names
        .iter()
        .flat_map(|v| partitions[v].found.iter().take(n).cloned())
        .collect::<Vec<_>>();
    let coverage = dict([
        (
            "subject_count",
            number(grouped.values().map(BTreeSet::len).sum()),
        ),
        ("match_count", number(found.len())),
        ("active_dataset_count", number(packs.len())),
        (
            "inactive_by_date_count",
            number(partitions.values().map(|p| p.inactive).sum()),
        ),
        (
            "lookup_queries",
            number(partitions.values().map(|p| p.lookups).sum()),
        ),
        ("match_limit", number(n)),
        ("match_limit_per_visibility", Value::Bool(true)),
        (
            "truncated",
            Value::Bool(partitions.values().any(|p| p.truncated)),
        ),
        (
            "visibility_coverage",
            dict(names.iter().map(|v| (*v, partitions[v].coverage(n)))),
        ),
        ("as_of", s(instant)),
        ("complete_chain_coverage", Value::Bool(false)),
        ("authority", s("imported_attribution_claims")),
    ]);
    Ok(dict([
        ("claims", Value::List(found)),
        ("datasets", Value::List(packs)),
        ("dataset_state_digest", s(state)),
        ("coverage", coverage),
    ]))
}

#[cfg(any(test, feature = "test-harness"))]
pub fn evaluate_fixture(case: &Value, snapshots: &Value) -> Value {
    let result = if text(case, "group") == "canonical" {
        let parts = list(field(case, "subject"));
        canonical_subject(&parts[0], &parts[1], &parts[2])
    } else {
        let snapshot = field(snapshots, text(case, "snapshot"));
        match_captured(
            field(snapshot, "packs"),
            field(snapshot, "claims"),
            field(case, "subjects"),
            get(case, "observer")
                .and_then(Value::as_str)
                .unwrap_or("owner"),
            get(case, "limit").unwrap_or(&number(5000)),
            get(case, "as_of").unwrap_or(&s("2026-02-01T00:00:00Z")),
        )
    };
    result.unwrap_or_else(|e| {
        dict([(
            "error",
            dict([
                ("code", s(e.code)),
                ("message", s(e.message)),
                ("details", e.details.unwrap_or(Value::Null)),
                ("retryable", Value::Bool(e.retryable)),
            ]),
        )])
    })
}

#[cfg(test)]
mod tests;
