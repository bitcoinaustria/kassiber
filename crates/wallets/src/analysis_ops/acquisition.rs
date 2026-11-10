//! Explicit, bounded reference acquisition (`chain_analysis_acquisition`).
//! Planning is local; applying recomputes the plan before any egress.
use super::routing::{get_db_backend, lower, py_text, routing_identity};
use super::sources::is_txid;
use super::*;
use crate::analysis_index::{durable, traversal::resolve_read, views, views::GraphRead};
use crate::descriptors::normalize_network;
use kassiber_setup::networks;

mod apply;
mod legs;
mod reader;
mod runner;
pub(super) use super::sources::MAX_RESPONSE_BYTES;
pub(super) use apply::apply;
pub(super) use runner::run_due;

pub(super) const MAX_TOTAL_BYTES: i64 = 64 * 1024 * 1024;

/// Bitcoin Core v31 and Elements consensus genesis hashes.
pub(super) fn genesis(chain: &str, network: &str) -> Option<&'static str> {
    Some(match (chain, network) {
        ("bitcoin", "main") => "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
        ("bitcoin", "test") => "000000000933ea01ad0ee984209779baaec3ced90fa3f408719526f8d77f4943",
        ("bitcoin", "signet") => "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6",
        ("bitcoin", "regtest") => {
            "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206"
        }
        ("liquid", "main") => "1466275836220db2944ca059a3a10ef6fd2ea684b0688d2c379296888a206003",
        _ => return None,
    })
}

fn network_of(chain: &str, network: &str) -> Result<String> {
    normalize_network(chain, network).map_err(|error| match error {
        crate::descriptors::Error::Value(message) => AppError::new("internal_error", message),
        crate::descriptors::Error::App(error) => error,
    })
}

fn bounded(value: &Value, maximum: i64) -> bool {
    matches!(value, Value::Int(n) if *n >= 1.into() && *n <= maximum.into())
}

/// `_prepare_acquisition`: the reviewed plan, the exact backend row and seeds.
pub(super) fn prepare(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<(Value, Value, Vec<String>)> {
    arguments(
        args,
        &[
            "backend",
            "subject",
            "chain",
            "network",
            "direction",
            "depth",
            "max_transactions",
            "genesis_hash",
        ],
        &["backend", "subject", "chain", "network", "direction"],
    )?;
    let (chain, network) = (field(args, "chain"), field(args, "network"));
    let domain_ok = matches!(chain, Value::Str(c) if matches!(c.as_str(), Some("bitcoin" | "liquid")))
        && matches!(network, Value::Str(n) if matches!(n.as_str(), Some("main" | "test" | "signet" | "regtest")))
        && !(chain == &s("liquid") && network == &s("signet"));
    if !domain_ok {
        return Err(invalid("A valid explicit chain and network are required"));
    }
    let (chain, network) = (py_text(chain), py_text(network));
    let mut value = args.clone();
    let backend_name = lower(&py_text(&Value::Str(text_value(
        field(args, "backend"),
        "backend",
        128,
    )?)));
    set(&mut value, "backend", s(backend_name.as_str()));
    let subject = text_value(field(args, "subject"), "subject", 256)?;
    set(&mut value, "subject", Value::Str(subject.clone()));
    for (key, default) in [("depth", 3), ("max_transactions", 50)] {
        let current = get(args, key)
            .cloned()
            .unwrap_or(Value::Int(default.into()));
        set(&mut value, key, current);
    }
    if !matches!(field(&value, "direction"), Value::Str(d) if matches!(d.as_str(), Some("backward" | "forward" | "both")))
    {
        return Err(invalid("Invalid acquisition direction"));
    }
    for (key, maximum) in [("depth", 10), ("max_transactions", 200)] {
        if !bounded(field(&value, key), maximum) {
            return Err(invalid(&format!("{key} must be between 1 and {maximum}")));
        }
    }
    let consensus = genesis(&chain, &network);
    let supplied = get(&value, "genesis_hash").unwrap_or(&Value::Null);
    if let Some(consensus) = consensus {
        if supplied != &Value::Null && supplied != &s(consensus) {
            return Err(invalid(
                "The supplied genesis hash conflicts with the selected standard network",
            ));
        }
    }
    let expected = consensus.map_or_else(|| supplied.clone(), s);
    if !is_txid(&expected) {
        return Err(invalid(
            "This custom network requires an explicit genesis_hash pin",
        ));
    }
    set(&mut value, "genesis_hash", expected);
    let backend = get_db_backend(conn, &backend_name)?;
    let kind = py_text(field(&backend, "kind"));
    if !["bitcoinrpc", "esplora", "liquid-esplora", "electrum"].contains(&kind.as_str()) {
        return Err(AppError::new(
            "capability_unavailable",
            "Selected backend cannot supply chain observations",
        ));
    }
    let backend_network = match get(&backend, "network") {
        None | Some(Value::Null) => String::new(),
        Some(other) => py_text(other),
    };
    if !py_eq(field(&backend, "chain"), &s(chain.as_str()))
        || network_of(&chain, &backend_network)? != network_of(&chain, &network)?
    {
        return Err(invalid(
            "Selected backend belongs to a different chain or network",
        ));
    }
    if kind == "bitcoinrpc" && chain != "bitcoin" {
        return Err(AppError::new(
            "capability_unavailable",
            "The Bitcoin Core observer supports Bitcoin only",
        ));
    }
    let book = networks::resolve(conn, profile)?;
    let domain = networks::require_chain_domain(
        conn,
        profile,
        &chain,
        &s(network.as_str()),
        None,
        "acquisition",
    )?;
    let instance = field(&backend, "chain_instance_id");
    if truthy(instance) && !py_eq(instance, field(&domain, "chain_instance_id")) {
        return Err(invalid(
            "This backend is assigned to a different local chain instance",
        ));
    }
    let normalized = network_of(&chain, &network)?;
    let (snapshot_id, seeds) = atomic(conn, || {
        durable::synchronize_with_sources(conn, profile, false, sources)?;
        views::with_read(conn, profile, "owner", |view| {
            let original = py_text(&Value::Str(subject.clone()));
            let subject = lower(&original);
            let bare = subject
                .split_once(":tx:")
                .map_or(subject.as_str(), |(_, rest)| rest);
            let bare = bare.split_once(":out:").map_or(bare, |(_, rest)| rest);
            let bare = bare.split(':').next().unwrap_or("");
            let outpoint = subject.split_once(':').is_some_and(|(txid, n)| {
                is_txid(&s(txid)) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
            });
            let seeds = if is_txid(&s(bare))
                && (subject == bare
                    || subject.starts_with(&format!("{chain}:{normalized}:"))
                    || outpoint)
            {
                vec![bare.to_owned()]
            } else {
                let mut seeds = std::collections::BTreeSet::new();
                for id in resolve_read(view, &original, &value)? {
                    let txid = field(&view.get("nodes", &id)?, "txid").clone();
                    if is_txid(&txid) {
                        seeds.insert(py_text(&txid));
                    }
                }
                seeds.into_iter().collect()
            };
            Ok((field(view.state(), "snapshot_id").clone(), seeds))
        })
    })?;
    if seeds.is_empty() {
        return Err(AppError::new(
            "not_found",
            "Acquisition needs a txid, outpoint or a locally resolved wallet/address",
        ));
    }
    let Value::Int(maximum) = field(&value, "max_transactions").clone() else {
        unreachable!("validated budget")
    };
    let maximum = i64::try_from(maximum).unwrap();
    if seeds.len() as i64 > maximum {
        return Err(invalid(
            "Subject has more transactions than this acquisition budget; narrow the subject",
        ));
    }
    let safe = dict(
        ["name", "kind", "chain", "network"]
            .map(|key| (key, get(&backend, key).cloned().unwrap_or(Value::Null))),
    );
    let mut limitations: Vec<Value> = [
        "Reference observations do not grant ownership or custody.",
        "Only the chosen backend is contacted; missing or pruned history remains a frontier.",
        "Confirmation and spend status are observations at acquisition time.",
        "The 45 second deadline stops new requests and response chunks; a pending network read has an inactivity timeout of at most 8 seconds.",
    ]
    .map(s)
    .to_vec();
    if kind == "electrum" {
        limitations.push(s("Electrum supplies ancestors, but no universal historical spender index or confirmation proof in this query."));
    }
    if kind == "bitcoinrpc" {
        limitations.push(s("Historical forward expansion requires a synchronized Core 31 txospenderindex; arbitrary raw transaction reads may require txindex."));
    }
    let book_instance = get(&book, "chain_instance_id")
        .cloned()
        .unwrap_or(Value::Null);
    if truthy(&book_instance) {
        limitations.push(s("Approval assigns the selected source to this local chain instance. Its genesis hash verifies the network, not the identity of a particular local chain history."));
    }
    let mut result = dict([
        ("args", value),
        ("snapshot_id", snapshot_id),
        ("backend", safe),
        (
            "effects",
            dict([
                ("egresses", Value::Bool(true)),
                ("max_transactions", Value::Int(maximum.into())),
                ("max_requests", Value::Int((6 + 2 * maximum).into())),
                ("scheduling_deadline_seconds", Value::Int(45.into())),
                ("request_timeout_seconds", Value::Int(8.into())),
                ("max_response_bytes", Value::Int(MAX_RESPONSE_BYTES.into())),
                (
                    "max_total_response_bytes",
                    Value::Int(MAX_TOTAL_BYTES.into()),
                ),
            ]),
        ),
        ("limitations", Value::List(limitations)),
        (
            "book_environment",
            dict([
                ("environment_id", field(&book, "environment_id").clone()),
                ("revision", field(&book, "revision").clone()),
                ("chain_instance_id", book_instance),
            ]),
        ),
    ]);
    // A configuration change invalidates consent without exposing secrets.
    let revision = first(
        conn,
        "SELECT updated_at FROM backends WHERE name=?",
        params![sql(field(&backend, "name"), 1)?],
    )?
    .map(|row| field(&row, "updated_at").clone())
    .unwrap_or(Value::Null);
    let plan_id = digest(&Value::List(vec![
        s(profile),
        result.clone(),
        revision,
        Value::List(seeds.iter().map(|seed| s(seed.as_str())).collect()),
        routing_identity(&backend)?,
    ]))?;
    set(&mut result, "plan_id", s(plan_id.as_str()));
    Ok((result, backend, seeds))
}

pub(super) fn plan(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    Ok(prepare(conn, profile, args, sources)?.0)
}

#[cfg(test)]
mod tests;
