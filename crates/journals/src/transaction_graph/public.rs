//! `_enrich_reference_graph_raw`'s explicit public lookup: the backends the
//! user chose, best first, each asked within the lookup budget. Failures
//! degrade to the caller's graph with a warning, as in Python, and a fetched
//! reference grants no ownership. The transport is `kassiber-wallets`'.
use super::*;
use kassiber_setup::backends::GraphRuntime;
use kassiber_wallets::freshness::graph::public::Network;
use std::time::{Duration, Instant};

/// Kassiber's shipped convenience backends (`backends.DEFAULT_BACKENDS`).
const DEFAULT_BACKENDS: [&str; 4] = ["mempool", "fulcrum", "liquid", "liquid-blockstream"];
const BUDGET: Duration = Duration::from_secs(10);
const MAX_PREVTX_LOOKUPS: usize = 250;
const GRAPH_CACHE_SCHEMA_VERSION: i64 = 2;
/// `BACKEND_CONFIG_FIELDS`, which `_graph_backend_from_mapping` copies.
const CONFIG_FIELDS: [&str; 14] = [
    "certificate",
    "commando_peer_id",
    "cookiefile",
    "display_name",
    "insecure",
    "lightning_cli",
    "lightning_dir",
    "password",
    "rpc_file",
    "silent_payments",
    "silent_payment_scan_file",
    "silent_payment_scan_path",
    "username",
    "walletprefix",
];

/// What an explicit public lookup may use: the user's request authorizes
/// each selected backend through this transport.
pub struct Public<'a> {
    pub network: Network<'a>,
    pub runtime: GraphRuntime<'a>,
}

/// `normalize_backend_kind`.
fn backend_kind(kind: &Value) -> String {
    let value = lower(&strip(&text(kind)));
    match value.as_str() {
        "bitcoin-core" | "bitcoincore" | "core" => "bitcoinrpc".into(),
        "core-ln" | "core-lightning" => "coreln".into(),
        "liquid-esplora" | "mempool" => "esplora".into(),
        _ => value,
    }
}

/// `a or b`: the first truthy operand, else the last.
fn either<'a>(a: &'a Value, b: &'a Value) -> &'a Value {
    if truthy(a) {
        a
    } else {
        b
    }
}

fn setting(conn: &Connection, key: &str) -> Result<Value> {
    Ok(rows(conn, "SELECT value FROM settings WHERE key=?", [key])?
        .first()
        .map(|row| g(row, "value").clone())
        .unwrap_or(Value::Null))
}

/// `_graph_backend_from_row`.
fn from_row(row: &Value) -> Value {
    let mut backend = dict(
        [
            "name",
            "kind",
            "chain",
            "network",
            "url",
            "timeout",
            "batch_size",
            "tor_proxy",
        ]
        .map(|key| (key, g(row, key).clone())),
    );
    if let Value::Dict(pairs) = json_obj(g(row, "config_json")) {
        for (key, value) in pairs {
            if let Key::Str(name) = &key {
                set(&mut backend, &py_str(&Value::Str(name.clone())), value);
            }
        }
    }
    backend
}

/// `_graph_backend_from_mapping(name, backend)`.
fn from_mapping(name: &str, backend: &Value) -> Value {
    let mut candidate = dict([
        ("name", s(name)),
        ("kind", g(backend, "kind").clone()),
        ("chain", g(backend, "chain").clone()),
        ("network", g(backend, "network").clone()),
        ("url", g(backend, "url").clone()),
        ("timeout", g(backend, "timeout").clone()),
        ("batch_size", g(backend, "batch_size").clone()),
        (
            "tor_proxy",
            either(g(backend, "tor_proxy"), g(backend, "proxy")).clone(),
        ),
        ("auth_header", g(backend, "auth_header").clone()),
        ("token", g(backend, "token").clone()),
        ("certificate", g(backend, "certificate").clone()),
        ("insecure", g(backend, "insecure").clone()),
    ]);
    for key in CONFIG_FIELDS {
        if let Some(value) = get(backend, key).filter(|v| **v != Value::Null) {
            set(&mut candidate, key, value.clone());
        }
    }
    if let Value::Dict(pairs) = g(backend, "config") {
        for (key, value) in pairs {
            if let Key::Str(name) = key {
                set(
                    &mut candidate,
                    &py_str(&Value::Str(name.clone())),
                    value.clone(),
                );
            }
        }
    }
    candidate
}

/// `_graph_backend_matches(backend, chain, network)`.
fn matches(backend: &Value, chain: &str, network: &str) -> bool {
    let kind = backend_kind(g(backend, "kind"));
    ["esplora", "mempool", "electrum", "bitcoinrpc"].contains(&kind.as_str())
        && string_or_none(g(backend, "url")).is_some()
        && norm_chain_network(g(backend, "chain"), g(backend, "network"))
            == norm_chain_network(&s(chain), &s(network))
}

/// `urlsplit(url)` for an http(s) URL: (scheme, netloc, path).
fn split(url: &str) -> Option<(String, String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if !["http", "https"].contains(&scheme.as_str()) {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (netloc, rest) = rest.split_at(end);
    let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
    (!netloc.is_empty()).then(|| (scheme, netloc.to_owned(), rest[..path_end].to_owned()))
}

/// `backends._http_url_base(url, api=True)`.
fn api_base(url: &Value) -> Option<String> {
    let (scheme, netloc, path) = split(&string_or_none(url)?)?;
    let mut path = path.trim_end_matches('/').to_owned();
    if !lower(&path).ends_with("/api") {
        path = if path.is_empty() {
            "/api".into()
        } else {
            format!("{path}/api")
        };
    }
    Some(format!("{scheme}://{netloc}{path}"))
}

/// Whether `backends._http_url_base(url, api=False)` has a base: a host and a
/// valid port.
fn explorer_base(url: &Value) -> bool {
    let Some((_, netloc, _)) = string_or_none(url).as_deref().and_then(split) else {
        return false;
    };
    let host = netloc.rsplit_once('@').map_or(netloc.as_str(), |(_, h)| h);
    let (hostname, port) = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some((name, port)) => (name, port.strip_prefix(':')),
            None => return false,
        }
    } else {
        match host.rsplit_once(':') {
            Some((name, port)) => (name, Some(port)),
            None => (host, None),
        }
    };
    !hostname.is_empty() && port.is_none_or(|p| p.is_empty() || p.parse::<u16>().is_ok())
}

/// `_normalized_graph_lookup_backend`.
fn normalized(mut backend: Value) -> Value {
    if ["esplora", "mempool"].contains(&backend_kind(g(&backend, "kind")).as_str()) {
        if let Some(url) = api_base(g(&backend, "url")) {
            set(&mut backend, "url", s(url.as_str()));
        }
    }
    backend
}

/// `_runtime_default_choice_is_explicit(runtime_config, name)`.
fn default_choice_is_explicit(runtime: &GraphRuntime<'_>, name: &str) -> bool {
    let name = lower(&strip(name));
    let bootstrap = lower(&strip(runtime.bootstrap_default));
    if !name.is_empty() && !bootstrap.is_empty() && name != bootstrap {
        return true;
    }
    !runtime.builtin_default || runtime.env_default
}

/// `_implicit_builtin_runtime_graph_backend(runtime_config, name, backend)`.
fn implicit_builtin(runtime: &GraphRuntime<'_>, name: &str, backend: &Value) -> bool {
    let name = lower(&strip(name));
    DEFAULT_BACKENDS.contains(&name.as_str())
        && !runtime.dotenv.contains(&name)
        && !runtime.env_backends.contains(name.as_str())
        && *g(backend, "source") == s("built-in default")
}

/// `_explicitly_chosen_default_backend_name(conn, runtime_config)`.
fn explicit_default(conn: &Connection, runtime: &GraphRuntime<'_>) -> Result<Option<String>> {
    let stored = string_or_none(&setting(conn, "default_backend")?);
    let bootstrap = string_or_none(&setting(conn, "bootstrap_default_backend")?);
    if let Some(stored) = stored {
        if bootstrap.is_none_or(|b| lower(&stored) != lower(&b)) {
            return Ok(Some(lower(&stored)));
        }
    }
    Ok(string_or_none(&s(runtime.default_backend))
        .filter(|name| default_choice_is_explicit(runtime, name))
        .map(|name| lower(&name)))
}

/// `_is_user_provided_graph_backend(backend, runtime_config, chosen_names=...)`.
fn user_provided(backend: &Value, runtime: &GraphRuntime<'_>, chosen: &[String]) -> bool {
    let name = string_or_none(g(backend, "name"))
        .map(|n| lower(&n))
        .unwrap_or_default();
    if !DEFAULT_BACKENDS.contains(&name.as_str()) || chosen.contains(&name) {
        return true;
    }
    let mut owner = g(backend, "infrastructure_owner");
    if *owner == Value::Null && is_dict(g(backend, "config")) {
        owner = g(g(backend, "config"), "infrastructure_owner");
    }
    lower(&strip(&text_or(owner))) == "self"
        || runtime.dotenv.contains(&name)
        || runtime.env_backends.contains(name.as_str())
}

/// `_runtime_graph_lookup_backends(runtime_config, chain, network)`.
fn runtime_backends(runtime: &GraphRuntime<'_>, chain: &str, network: &str) -> Vec<Value> {
    let default_name = lower(&strip(runtime.default_backend));
    let mut names: Vec<&str> = Vec::new();
    if !default_name.is_empty() {
        names.push(&default_name);
    }
    for name in runtime.backends.keys() {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    let mut default_candidate = None;
    let mut candidates = Vec::new();
    for name in names {
        let Some(backend) = runtime.backends.get(name).filter(|b| is_dict(b)) else {
            continue;
        };
        let candidate = from_mapping(name, backend);
        if !matches(&candidate, chain, network) {
            continue;
        }
        if lower(&strip(name)) == default_name {
            if !implicit_builtin(runtime, name, backend)
                || default_choice_is_explicit(runtime, name)
            {
                candidates.push(candidate);
                default_candidate = None;
            } else {
                default_candidate = Some(candidate);
            }
            continue;
        }
        if implicit_builtin(runtime, name, backend) {
            continue;
        }
        candidates.push(candidate);
    }
    candidates.extend(default_candidate);
    candidates.into_iter().map(normalized).collect()
}

const BACKEND_COLUMNS: &str =
    "SELECT name, kind, chain, network, url, timeout, batch_size, tor_proxy, config_json FROM backends";

/// `_wallet_graph_backend` / `_default_graph_backend`: one named row.
fn named(conn: &Connection, name: &str, chain: &str, network: &str) -> Result<Option<Value>> {
    let row = rows(
        conn,
        &format!("{BACKEND_COLUMNS} WHERE lower(name) = ? LIMIT 1"),
        [name],
    )?;
    Ok(row
        .first()
        .map(from_row)
        .filter(|backend| matches(backend, chain, network)))
}

/// `backends.backend_value(backend, key)`.
fn backend_value(backend: &Value, key: &str) -> Option<String> {
    string_or_none(g(backend, key))
}

/// `_backend_matches_chain_network(backend, chain, network)`; an unsupported
/// stored chain or network does not match.
fn explorer_matches(backend: &Value, chain: &str, network: &str) -> bool {
    let (Some(wanted_chain), Some(backend_chain)) = (
        normalize_chain(chain),
        normalize_chain(&backend_value(backend, "chain").unwrap_or_else(|| "bitcoin".into())),
    ) else {
        return false;
    };
    let fallback = if backend_chain == "liquid" {
        "liquidv1"
    } else {
        "main"
    };
    let wanted_network = normalize_network(wanted_chain, network);
    let backend_network = normalize_network(
        backend_chain,
        &backend_value(backend, "network").unwrap_or_else(|| fallback.into()),
    );
    backend_chain == wanted_chain && backend_network.is_some() && backend_network == wanted_network
}

/// `_preferred_mempool_graph_backend`: the preferred HTTP explorer, as
/// `preferred_mempool_api_backend` ranks them.
fn preferred_explorer(conn: &Connection, chain: &str, network: &str) -> Result<Option<Value>> {
    let default_name = text_or(&setting(conn, "default_backend")?);
    let mut candidates = Vec::new();
    for row in rows(conn, "SELECT * FROM backends ORDER BY name ASC", [])? {
        let mut backend = row.clone();
        if let Value::Dict(pairs) = json_obj(g(&row, "config_json")) {
            for (key, value) in pairs {
                if let Key::Str(name) = &key {
                    set(&mut backend, &py_str(&Value::Str(name.clone())), value);
                }
            }
        }
        let kind = lower(&backend_value(&backend, "kind").unwrap_or_default());
        if !["esplora", "mempool", "liquid-esplora"].contains(&kind.as_str())
            || !explorer_matches(&backend, chain, network)
        {
            continue;
        }
        let Some(api) = api_base(&s(backend_value(&backend, "url").unwrap_or_default()))
            .filter(|_| explorer_base(&s(backend_value(&backend, "url").unwrap_or_default())))
        else {
            continue;
        };
        let name = text(g(&backend, "name"));
        let own = backend_value(&backend, "infrastructure_owner").as_deref() == Some("self");
        candidates.push((
            (
                !own,
                DEFAULT_BACKENDS.contains(&name.as_str()),
                !(!default_name.is_empty() && name == default_name),
                name.clone(),
            ),
            name,
            api,
            backend,
        ));
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(candidates
        .into_iter()
        .next()
        .map(|(_, name, api, backend)| {
            let timeout = backend_value(&backend, "timeout")
                .and_then(|t| kassiber_pytext::parse_int(&t).ok())
                .map_or(Value::Null, Value::Int);
            dict([
                ("name", s(name.as_str())),
                ("kind", s("esplora")),
                ("chain", s(chain)),
                ("network", s(network)),
                ("url", s(api.as_str())),
                ("timeout", timeout),
                (
                    "tor_proxy",
                    backend_value(&backend, "tor_proxy").map_or(Value::Null, |p| s(p.as_str())),
                ),
            ])
        }))
}

/// `_graph_lookup_backends(conn, row, runtime_config, ...)`: the wallet's own
/// backend, the user's runtime backends, the chosen default, the preferred
/// explorer and every configured row, without shipped defaults the user
/// never chose.
fn backends(
    conn: &Connection,
    row: &Value,
    runtime: &GraphRuntime<'_>,
    default_chain: &str,
    default_network: &str,
) -> Result<Vec<Value>> {
    let (chain, network) = row_chain_network(row, default_chain, default_network);
    let wallet_backend = string_or_none(g(&json_obj(g(row, "wallet_config_json")), "backend"))
        .map(|name| lower(&name));
    let mut chosen: Vec<String> = Vec::new();
    chosen.extend(wallet_backend.clone());
    chosen.extend(explicit_default(conn, runtime)?);
    let mut candidates: Vec<Value> = Vec::new();
    let mut seen: Vec<(String, String, String, String)> = Vec::new();
    let mut append = |candidate: Option<Value>| {
        let Some(candidate) = candidate else { return };
        if !user_provided(&candidate, runtime, &chosen) {
            return;
        }
        let candidate = normalized(candidate);
        let kind = backend_kind(g(&candidate, "kind"));
        let identity = (
            if ["esplora", "mempool"].contains(&kind.as_str()) {
                "http".to_owned()
            } else {
                kind
            },
            text_or(g(&candidate, "chain")),
            text_or(g(&candidate, "network")),
            text_or(g(&candidate, "url")),
        );
        if seen.contains(&identity) {
            return;
        }
        seen.push(identity);
        candidates.push(candidate);
    };
    if let Some(name) = &wallet_backend {
        append(named(conn, name, &chain, &network)?);
    }
    for candidate in runtime_backends(runtime, &chain, &network) {
        append(Some(candidate));
    }
    if let Some(name) = string_or_none(&setting(conn, "default_backend")?) {
        append(named(conn, &lower(&name), &chain, &network)?.map(normalized));
    }
    append(preferred_explorer(conn, &chain, &network)?);
    let configured = rows(
        conn,
        &format!(
            "{BACKEND_COLUMNS} ORDER BY CASE lower(kind) WHEN 'esplora' THEN 0 WHEN 'mempool' THEN 0 \
             WHEN 'liquid-esplora' THEN 0 WHEN 'electrum' THEN 1 WHEN 'bitcoinrpc' THEN 2 ELSE 3 END, \
             updated_at DESC, name ASC"
        ),
        [],
    )?;
    for row in &configured {
        let backend = from_row(row);
        if matches(&backend, &chain, &network) {
            append(Some(normalized(backend)));
        }
    }
    Ok(candidates)
}

/// `_sanitize_graph_value_script(entry, chain, include_n=...)`.
fn value_script(entry: &Value, chain: &str, include_n: bool) -> Value {
    let mut clean = dict::<&str>([]);
    if include_n {
        if let Some(n) = int_or_none(g(entry, "n")) {
            set(&mut clean, "n", Value::Int(n.into()));
        }
    }
    if let Some(script) = string_or_none(&output_script(entry)) {
        set(&mut clean, "scriptpubkey", s(script.as_str()));
    }
    if let Some(kind) = string_or_none(either(g(entry, "scriptpubkey_type"), g(entry, "type"))) {
        set(&mut clean, "scriptpubkey_type", s(kind.as_str()));
    }
    if let Some(address) = output_address(entry) {
        set(&mut clean, "scriptpubkey_address", s(address.as_str()));
    }
    if let Some(asset) = leg_asset(entry, None) {
        set(&mut clean, "asset", s(asset.as_str()));
    }
    if let Some(address) = pegout_address(entry) {
        set(&mut clean, "pegout_address", s(address.as_str()));
    }
    let value = output_value_sats(entry);
    let mut probe = clean.clone();
    set(
        &mut probe,
        "value",
        value.map_or(Value::Null, |v| Value::Int(v.into())),
    );
    if confidential_leg(entry) || (chain == "liquid" && !is_liquid_fee_output(&probe)) {
        set(&mut clean, "value_state", s("confidential"));
    } else if let Some(value) = value {
        set(&mut clean, "value", Value::Int(value.into()));
    }
    clean
}

/// `_sanitize_graph_lookup_raw(raw, chain, txid)`.
fn sanitize(raw: &Value, chain: &str, txid: Option<&str>) -> Value {
    let mut clean = dict::<&str>([]);
    if let Some(id) = string_or_none(g(raw, "txid")).or_else(|| txid.map(str::to_owned)) {
        set(&mut clean, "txid", s(lower(&id).as_str()));
    }
    for key in ["version", "locktime", "size", "vsize", "weight"] {
        if let Some(value) = int_or_none(g(raw, key)) {
            set(&mut clean, key, Value::Int(value.into()));
        }
    }
    if let Some(vin) = list(g(raw, "vin")) {
        let entries = vin
            .iter()
            .filter(|entry| is_dict(entry))
            .map(|entry| {
                let mut leg = dict::<&str>([]);
                if let Some(id) = string_or_none(g(entry, "txid")) {
                    set(&mut leg, "txid", s(lower(&id).as_str()));
                }
                if let Some(vout) = int_or_none(g(entry, "vout")) {
                    set(&mut leg, "vout", Value::Int(vout.into()));
                }
                if is_pegin_leg(entry) {
                    set(&mut leg, "is_pegin", Value::Bool(true));
                }
                if is_coinbase_leg(entry) {
                    set(&mut leg, "is_coinbase", Value::Bool(true));
                }
                let prevout = g(entry, "prevout");
                if is_dict(prevout) {
                    let prevout = value_script(prevout, chain, false);
                    if truthy(&prevout) {
                        set(&mut leg, "prevout", prevout);
                    }
                }
                leg
            })
            .collect();
        set(&mut clean, "vin", Value::List(entries));
    }
    if let Some(vout) = list(g(raw, "vout")) {
        let entries = vout
            .iter()
            .enumerate()
            .filter(|(_, entry)| is_dict(entry))
            .map(|(index, entry)| {
                let mut leg = value_script(entry, chain, true);
                if *g(&leg, "n") == Value::Null {
                    set(&mut leg, "n", Value::Int(index.into()));
                }
                leg
            })
            .collect();
        set(&mut clean, "vout", Value::List(entries));
    }
    clean
}

/// `_store_graph_lookup_cache(conn, chain, network, txid, raw)`.
fn store(conn: &Connection, chain: &str, network: &str, txid: &str, raw: &Value) -> Result<Value> {
    let txid = lower(&strip(txid));
    let mut clean = sanitize(raw, chain, Some(&txid));
    if list(g(&clean, "vin")).is_none() || list(g(&clean, "vout")).is_none() {
        return Ok(clean);
    }
    let context = kassiber_setup::context::snapshot(conn)?;
    let profile_id = text_or(g(&context, "profile_id"));
    let binding = kassiber_setup::networks::resolve(conn, &profile_id)?;
    if *g(&binding, "state") == s("bound") {
        let domain = kassiber_setup::networks::require_chain_domain(
            conn,
            &profile_id,
            chain,
            &s(network),
            None,
            "graph_lookup",
        )?;
        if truthy(g(&domain, "chain_instance_id")) {
            set(
                &mut clean,
                "chain_instance_id",
                g(&domain, "chain_instance_id").clone(),
            );
        }
    }
    let payload = kassiber_pyjson::dumps(
        &kassiber_envelope::json_ready(clean.clone()),
        &kassiber_pyjson::Options::new()
            .sort_keys(true)
            .separators(",", ":"),
    )
    .map_err(|_| AppError::new("internal_error", "Graph reference cannot be encoded"))?;
    let now = kassiber_wallets::freshness::graph::public::now_iso();
    conn.execute(
        "INSERT INTO transaction_graph_cache(schema_version, chain, network, txid, payload_json, created_at, updated_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?) ON CONFLICT(schema_version, chain, network, txid) DO UPDATE SET \
         payload_json = excluded.payload_json, updated_at = excluded.updated_at",
        kassiber_store::rusqlite::params![GRAPH_CACHE_SCHEMA_VERSION, chain, network, txid, payload, now, now],
    )
    .map_err(sql_error)?;
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").map_err(sql_error)?;
    }
    Ok(clean)
}

/// `_load_graph_lookup_cache`.
fn cached(conn: &Connection, chain: &str, network: &str, txid: &str) -> Result<Option<Value>> {
    kassiber_wallets::freshness::graph::cached_reference(conn, chain, network, txid)?
        .map(|v| graph_wire(&v))
        .transpose()
}

/// `_prevout_from_cached_graph(raw, index)`.
fn cached_prevout(raw: &Value, index: i128) -> Option<Value> {
    let entry = list(g(raw, "vout"))?
        .iter()
        .filter(|entry| is_dict(entry))
        .find(|entry| int_or_none(g(entry, "n")) == Some(index))?;
    let mut prevout = dict::<&str>([]);
    for key in [
        "scriptpubkey",
        "scriptpubkey_type",
        "scriptpubkey_address",
        "value",
        "value_state",
    ] {
        if *g(entry, key) != Value::Null {
            set(&mut prevout, key, g(entry, key).clone());
        }
    }
    Some(prevout)
}

/// `_attach_bitcoin_prevouts_from_cached_graphs(raw, cached_prev)`.
fn attach_cached(raw: &mut Value, cached: &[(String, Value)]) {
    let mut vin = match g(raw, "vin") {
        Value::List(items) => items.clone(),
        _ => return,
    };
    for entry in vin.iter_mut().filter(|entry| is_dict(entry)) {
        let (Some(id), Some(index)) = (
            string_or_none(g(entry, "txid")),
            int_or_none(g(entry, "vout")),
        ) else {
            continue;
        };
        let Some((_, previous)) = cached.iter().find(|(key, _)| *key == lower(&id)) else {
            continue;
        };
        if let Some(prevout) = cached_prevout(previous, index) {
            set(entry, "prevout", prevout);
        }
    }
    set(raw, "vin", Value::List(vin));
}

/// `fetch_missing`: the previous transactions' graphs, by txid.
type FetchMissing<'a> = dyn FnMut(&[String]) -> Result<Vec<(String, Value)>> + 'a;

/// `_attach_bitcoin_prevouts(conn, chain, network, raw, fetch_missing=..., source_label=...)`.
fn attach_prevouts(
    conn: &Connection,
    chain: &str,
    network: &str,
    raw: &Value,
    fetch_missing: &mut FetchMissing<'_>,
    source: &str,
) -> Result<Value> {
    let mut enriched = raw.clone();
    let vin: Vec<Value> = list(g(raw, "vin"))
        .map(|items| items.iter().filter(|e| is_dict(e)).cloned().collect())
        .unwrap_or_default();
    set(&mut enriched, "vin", Value::List(vin.clone()));
    let mut missing: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut previous: Vec<(String, Value)> = Vec::new();
    for entry in &vin {
        let (Some(id), Some(_)) = (
            string_or_none(g(entry, "txid")),
            int_or_none(g(entry, "vout")),
        ) else {
            continue;
        };
        if !looks_like_txid(&s(id.as_str())) || !needs_prevout_lookup(entry) {
            continue;
        }
        let id = lower(&id);
        if seen.contains(&id) {
            continue;
        }
        seen.push(id.clone());
        match cached(conn, chain, network, &id)? {
            Some(raw) => previous.push((id, raw)),
            None => missing.push(id),
        }
    }
    if missing.len() > MAX_PREVTX_LOOKUPS {
        attach_cached(&mut enriched, &previous);
        return Ok(with_lookup_warning(
            &enriched,
            "bitcoin_reference_lookup_prevout_limit",
            &format!(
                "{source} graph lookup needs too many uncached previous transactions; Kassiber capped the request to avoid flooding the {source} backend."
            ),
        ));
    }
    if !missing.is_empty() {
        for (id, raw) in fetch_missing(&missing)? {
            let stored = store(conn, chain, network, &id, &raw)?;
            previous.push((id, stored));
        }
    }
    attach_cached(&mut enriched, &previous);
    Ok(enriched)
}

/// `_fetch_bitcoin_electrum_graph_raw` / `_fetch_liquid_electrum_graph_raw`.
#[allow(clippy::too_many_arguments)]
fn electrum(
    conn: &Connection,
    public: &Public<'_>,
    backend: &Value,
    chain: &str,
    network: &str,
    txid: &str,
    liquid: bool,
    deadline: Instant,
) -> Result<Value> {
    let mut client = public.network.electrum(&serde(backend), deadline)?;
    if liquid {
        let raw = client.liquid(txid)?;
        return store(conn, chain, network, txid, &raw);
    }
    let current = client.bitcoin(txid)?;
    let raw = attach_prevouts(
        conn,
        chain,
        network,
        &current,
        &mut |missing| client.parents(missing),
        "Bitcoin Electrum",
    )?;
    if !has_required_prevouts(&raw) {
        if truthy(g(&raw, "_graphLookupWarning")) {
            return Ok(raw);
        }
        return Ok(with_lookup_warning(
            &raw,
            "bitcoin_reference_lookup_incomplete",
            "Bitcoin Electrum lookup did not return every previous output needed for a complete graph.",
        ));
    }
    store(conn, chain, network, txid, &raw)
}

/// `_bitcoinrpc_vout_to_graph`.
fn core_vout(source: &Value) -> Value {
    let empty = dict::<&str>([]);
    let script = if is_dict(g(source, "scriptPubKey")) {
        g(source, "scriptPubKey")
    } else {
        &empty
    };
    let address = string_or_none(g(script, "address")).or_else(|| {
        list(g(script, "addresses"))
            .and_then(|items| items.first())
            .and_then(string_or_none)
    });
    let mut payload = dict([
        ("n", g(source, "n").clone()),
        ("value", g(source, "value").clone()),
        (
            "scriptpubkey",
            either(g(script, "hex"), g(source, "scriptpubkey")).clone(),
        ),
        (
            "scriptpubkey_type",
            either(g(script, "type"), g(source, "scriptpubkey_type")).clone(),
        ),
    ]);
    if let Some(address) = address {
        set(&mut payload, "scriptpubkey_address", s(address.as_str()));
    }
    payload
}

/// `_bitcoinrpc_decoded_to_graph_raw`. Core inlines `prevout` only at
/// verbosity 2; otherwise `attach_prevouts` resolves the parents.
fn core_graph(decoded: &Value) -> Value {
    let entries = |key: &str| -> Vec<Value> {
        list(g(decoded, key))
            .map(|items| items.iter().filter(|e| is_dict(e)).cloned().collect())
            .unwrap_or_default()
    };
    let vin = entries("vin")
        .iter()
        .map(|entry| {
            let mut input = dict([
                ("txid", g(entry, "txid").clone()),
                ("vout", g(entry, "vout").clone()),
                ("sequence", g(entry, "sequence").clone()),
            ]);
            if matches!(g(entry, "coinbase"), Value::Str(text) if text.as_str() != Some("")) {
                set(&mut input, "is_coinbase", Value::Bool(true));
            }
            if is_dict(g(entry, "prevout")) {
                set(&mut input, "prevout", core_vout(g(entry, "prevout")));
            }
            input
        })
        .collect();
    let vout = entries("vout").iter().map(core_vout).collect();
    dict([
        (
            "txid",
            string_or_none(g(decoded, "txid")).map_or(Value::Null, |id| s(id.as_str())),
        ),
        ("version", g(decoded, "version").clone()),
        ("locktime", g(decoded, "locktime").clone()),
        ("size", g(decoded, "size").clone()),
        ("vsize", g(decoded, "vsize").clone()),
        ("weight", g(decoded, "weight").clone()),
        ("raw_hex", g(decoded, "hex").clone()),
        ("vin", Value::List(vin)),
        ("vout", Value::List(vout)),
    ])
}

/// `_fetch_bitcoinrpc_transaction_graph`: the transaction, then each
/// missing parent once. A pruned or missing parent (`-5`) does not make the
/// others useless; any other failure stops asking that backend.
fn core(
    conn: &Connection,
    public: &Public<'_>,
    backend: &Value,
    chain: &str,
    network: &str,
    txid: &str,
    deadline: Instant,
) -> Result<Value> {
    let client = public.network.core(&serde(backend), deadline)?;
    let decoded = client.transaction(txid)?;
    if !is_dict(&decoded) {
        return Err(AppError::new(
            "app_error",
            "Bitcoin Core returned an invalid transaction response",
        ));
    }
    let raw = attach_prevouts(
        conn,
        chain,
        network,
        &core_graph(&decoded),
        &mut |missing| {
            let mut fetched = Vec::new();
            for id in missing {
                if Instant::now() >= client.deadline() {
                    break;
                }
                match client.transaction(id) {
                    Ok(previous) if is_dict(&previous) => {
                        fetched.push((id.clone(), core_graph(&previous)))
                    }
                    Ok(_) => {}
                    Err(error)
                        if error
                            .details
                            .as_ref()
                            .and_then(|d| get(d, "rpc_error_code"))
                            == Some(&Value::Int((-5).into())) => {}
                    Err(_) => break,
                }
            }
            Ok(fetched)
        },
        "Bitcoin Core",
    )?;
    if !has_required_prevouts(&raw) {
        if truthy(g(&raw, "_graphLookupWarning")) {
            return Ok(raw);
        }
        return Ok(with_lookup_warning(
            &raw,
            "bitcoin_reference_lookup_incomplete",
            "Bitcoin Core lookup did not return every previous output needed for a complete graph.",
        ));
    }
    store(conn, chain, network, txid, &raw)
}

/// `_fetch_reference_graph_from_backend`.
#[allow(clippy::too_many_arguments)]
fn from_backend(
    conn: &Connection,
    public: &Public<'_>,
    backend: &Value,
    chain: &str,
    network: &str,
    txid: &str,
    raw: &Value,
    liquid: bool,
    deadline: Instant,
) -> Result<Value> {
    let (label, prefix) = if liquid {
        ("Liquid", "liquid")
    } else {
        ("Bitcoin", "bitcoin")
    };
    let warning = |base: &Value, code: &str, message: String| {
        with_lookup_warning(base, &format!("{prefix}_{code}"), &message)
    };
    let kind = backend_kind(g(backend, "kind"));
    // The transport reads the normalized kind, as Python's dispatch does.
    let mut backend = backend.clone();
    set(&mut backend, "kind", s(kind.as_str()));
    let backend = &backend;
    if kind == "electrum" {
        return Ok(
            electrum(conn, public, backend, chain, network, txid, liquid, deadline).unwrap_or_else(
                |_| {
                    warning(
                        raw,
                        "reference_lookup_failed_electrum",
                        format!("Could not fetch public {label} transaction references from a configured Electrum backend."),
                    )
                },
            ),
        );
    }
    if kind == "bitcoinrpc" && !liquid {
        return Ok(
            core(conn, public, backend, chain, network, txid, deadline).unwrap_or_else(|_| {
                with_lookup_warning(
                    raw,
                    "bitcoin_reference_lookup_failed_bitcoinrpc",
                    "Could not fetch public Bitcoin transaction references from a configured Bitcoin Core backend.",
                )
            }),
        );
    }
    let Ok(fetched) = public.network.esplora(&serde(backend), txid, deadline) else {
        return Ok(warning(
            raw,
            "reference_lookup_failed_explorer",
            format!("Could not fetch public {label} transaction references from a configured explorer backend."),
        ));
    };
    if !is_dict(&fetched) {
        return Ok(warning(
            raw,
            "reference_lookup_invalid",
            format!(
                "A configured {label} explorer backend returned an invalid transaction response."
            ),
        ));
    }
    let clean = sanitize(&fetched, chain, Some(txid));
    if string_or_none(g(&clean, "txid")).is_some_and(|id| lower(&id) != lower(txid)) {
        return Ok(warning(
            raw,
            "reference_lookup_mismatch",
            format!("{label} reference lookup returned a different transaction id."),
        ));
    }
    if list(g(&clean, "vin")).is_none() || list(g(&clean, "vout")).is_none() {
        return Ok(warning(
            raw,
            "reference_lookup_incomplete",
            format!("{label} reference lookup did not return public input/output references."),
        ));
    }
    if !liquid && !has_required_prevouts(&clean) {
        return Ok(with_lookup_warning(
            &clean,
            "bitcoin_reference_lookup_incomplete",
            "Bitcoin reference lookup did not return every previous output needed for a complete graph.",
        ));
    }
    store(conn, chain, network, txid, &clean)
}

/// The public half of `_enrich_reference_graph_raw`: every selected backend
/// in order within the budget; the first complete graph wins, else the last
/// warning, else a failure warning on the caller's graph.
#[allow(clippy::too_many_arguments)]
pub(super) fn lookup(
    conn: &Connection,
    public: &Public<'_>,
    row: &Value,
    raw: &Value,
    chain: &str,
    network: &str,
    txid: &str,
    liquid: bool,
) -> Result<Value> {
    let (label, prefix) = if liquid {
        ("Liquid", "liquid")
    } else {
        ("Bitcoin", "bitcoin")
    };
    let candidates = if liquid {
        if row_chain_network(row, "liquid", "liquidv1").0 != "liquid" {
            Vec::new()
        } else {
            backends(conn, row, &public.runtime, "liquid", "liquidv1")?
        }
    } else {
        backends(conn, row, &public.runtime, "bitcoin", "main")?
    };
    if candidates.is_empty() {
        return Ok(with_lookup_warning(
            raw,
            &format!("{prefix}_reference_lookup_unavailable"),
            &format!("No configured {label} graph backend is available to fetch public transaction references."),
        ));
    }
    let deadline = Instant::now() + BUDGET;
    let mut fallback = None;
    for backend in &candidates {
        if Instant::now() >= deadline {
            break;
        }
        let fetched = from_backend(
            conn, public, backend, chain, network, txid, raw, liquid, deadline,
        )?;
        if truthy(g(&fetched, "_graphLookupWarning")) {
            fallback = Some(fetched);
            continue;
        }
        return Ok(fetched);
    }
    Ok(fallback.unwrap_or_else(|| {
        with_lookup_warning(
            raw,
            &format!("{prefix}_reference_lookup_failed"),
            &format!("Could not fetch {label} transaction references from any configured backend."),
        )
    }))
}
