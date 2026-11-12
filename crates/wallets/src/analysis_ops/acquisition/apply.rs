//! `apply_acquisition`: recompute the reviewed plan, fetch within its budget
//! and save sanitized reference observations atomically. Observations never
//! grant ownership, custody or tax authority.
use super::super::routing::{backend_timeout, py_text};
use super::super::*;
use super::legs::*;
use super::reader::{Budget, Reader};
use crate::descriptors::normalize_network;
use kassiber_pyjson::BigInt;
use std::collections::{BTreeSet, VecDeque};

fn fail(code: &str, message: &str) -> AppError {
    AppError::new(code, message)
}

fn txid_text(value: &Value) -> Option<String> {
    is_txid(value).then(|| py_text(value))
}

fn small(value: &Value) -> i64 {
    match value {
        Value::Int(n) => i64::try_from(n.clone()).unwrap_or(i64::MAX),
        _ => 0,
    }
}

/// Bitcoin Electrum bytes through the consensus decoder, like embit.
fn electrum_bitcoin(txid: &str, raw_hex: &str) -> Result<(Value, Value)> {
    use bitcoin::consensus::encode::deserialize;
    let invalid = || fail("invalid_observation", "Invalid transaction bytes");
    let bytes: Vec<u8> = bitcoin::hex::FromHex::from_hex(raw_hex).map_err(|_| {
        AppError::new(
            "internal_error",
            "non-hexadecimal number found in fromhex() arg",
        )
    })?;
    let tx: bitcoin::Transaction =
        deserialize::<bitcoin::Transaction>(&bytes).map_err(|_| invalid())?;
    let actual = tx.compute_txid().to_string();
    let mut inputs = Vec::new();
    let mut feature_inputs = Vec::new();
    for input in &tx.input {
        let prev = input.previous_output;
        let mut entry = dict::<&str>([]);
        if prev.is_null() {
            set(&mut entry, "is_coinbase", Value::Bool(true));
        } else {
            set(&mut entry, "txid", s(prev.txid.to_string().as_str()));
            set(&mut entry, "vout", int(prev.vout.into()));
        }
        inputs.push(entry);
        feature_inputs.push(serde_json::json!({
            "txid": prev.txid.to_string(), "vout": prev.vout, "sequence": input.sequence.0,
            "scriptsig": hex(input.script_sig.as_bytes()),
            "witness": input.witness.iter().map(hex).collect::<Vec<_>>(), "prevout": {}
        }));
    }
    let outputs: Vec<Value> = tx
        .output
        .iter()
        .enumerate()
        .map(|(n, output)| {
            let mut entry = dict([("n", int(n as i64))]);
            if !output.script_pubkey.is_empty() {
                set(
                    &mut entry,
                    "scriptpubkey",
                    s(hex(output.script_pubkey.as_bytes()).as_str()),
                );
            }
            set(
                &mut entry,
                "value",
                Value::Int(output.value.to_sat().into()),
            );
            entry
        })
        .collect();
    let mut raw = dict([
        ("txid", s(lower(txid).as_str())),
        ("version", int(i64::from(tx.version.0 as u32))),
        ("locktime", int(tx.lock_time.to_consensus_u32().into())),
        ("vin", Value::List(inputs)),
        ("vout", Value::List(outputs)),
    ]);
    set(&mut raw, "size", int(tx.total_size() as i64));
    set(&mut raw, "vsize", int(tx.vsize() as i64));
    set(&mut raw, "weight", int(tx.weight().to_wu() as i64));
    let feature_raw = serde_json::json!({
        "version": tx.version.0 as u32, "locktime": tx.lock_time.to_consensus_u32(),
        "vin": feature_inputs,
        "vout": tx.output.iter().map(|o| serde_json::json!({"value_sats": o.value.to_sat(), "scriptpubkey": hex(o.script_pubkey.as_bytes())})).collect::<Vec<_>>(),
    });
    if actual != txid {
        return Err(fail(
            "invalid_observation",
            "Backend returned different transaction bytes",
        ));
    }
    Ok((raw, from_serde(&feature_raw)))
}

/// Liquid Electrum bytes; values and assets stay confidential except the fee.
fn electrum_liquid(txid: &str, raw_hex: &str) -> Result<Value> {
    use elements_miniscript::elements::{confidential, encode::deserialize, Transaction};
    let bytes: Vec<u8> = bitcoin::hex::FromHex::from_hex(raw_hex).map_err(|_| {
        AppError::new(
            "internal_error",
            "non-hexadecimal number found in fromhex() arg",
        )
    })?;
    let tx: Transaction = deserialize(&bytes)
        .map_err(|_| fail("invalid_observation", "Invalid transaction bytes"))?;
    if tx.txid().to_string() != txid {
        return Err(fail(
            "invalid_observation",
            "Backend returned different transaction bytes",
        ));
    }
    let inputs = tx
        .input
        .iter()
        .map(|input| {
            let mut entry = dict([
                ("txid", s(input.previous_output.txid.to_string().as_str())),
                ("vout", int(input.previous_output.vout.into())),
                ("prevout", dict([("value_state", s("confidential"))])),
            ]);
            if input.is_pegin() {
                set(&mut entry, "is_pegin", Value::Bool(true));
            }
            entry
        })
        .collect();
    let outputs = tx
        .output
        .iter()
        .enumerate()
        .map(|(n, output)| {
            let mut entry = dict([("n", int(n as i64))]);
            let script = hex(output.script_pubkey.as_bytes());
            if !script.is_empty() {
                set(&mut entry, "scriptpubkey", s(script.as_str()));
                set(&mut entry, "value_state", s("confidential"));
            } else if let confidential::Value::Explicit(value) = output.value {
                set(&mut entry, "scriptpubkey_type", s("fee"));
                set(&mut entry, "value", Value::Int(value.into()));
            } else {
                set(&mut entry, "value_state", s("confidential"));
            }
            entry
        })
        .collect();
    Ok(dict([
        ("txid", s(lower(txid).as_str())),
        ("version", int(i64::from(tx.version))),
        ("locktime", int(tx.lock_time.to_consensus_u32().into())),
        ("vin", Value::List(inputs)),
        ("vout", Value::List(outputs)),
        ("size", int(bytes.len() as i64)),
    ]))
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn lower(text: &str) -> String {
    kassiber_pytext::lower(text)
}

/// `_Reader.transaction`: one sanitized observation and its source status.
pub(super) fn transaction(
    reader: &mut Reader<'_>,
    txid: &str,
    chain: &str,
) -> Result<(Value, Value)> {
    let mut status = dict::<&str>([]);
    let mut feature_raw = None;
    let raw = if reader.kind == "bitcoinrpc" {
        // Verbosity 2 retains prevout values and scripts when Core has undo data.
        let decoded = reader.rpc("getrawtransaction", Value::List(vec![s(txid), int(2)]))?;
        if !matches!(decoded, Value::Dict(_)) || field(&decoded, "txid") != &s(txid) {
            return Err(fail(
                "invalid_observation",
                "Backend returned a different transaction",
            ));
        }
        let confirmations = field(&decoded, "confirmations");
        let count = match confirmations {
            Value::Int(n) => Some(n.clone()),
            _ => None,
        };
        set(
            &mut status,
            "confirmed",
            Value::Bool(count.as_ref().is_some_and(|n| *n > BigInt::from(0))),
        );
        set(
            &mut status,
            "conflicted",
            Value::Bool(count.as_ref().is_some_and(|n| *n < BigInt::from(0))),
        );
        if let Some(n) = count.filter(|n| *n >= BigInt::from(0)) {
            set(&mut status, "confirmations", Value::Int(n));
        }
        if is_txid(field(&decoded, "blockhash")) {
            set(
                &mut status,
                "block_hash",
                field(&decoded, "blockhash").clone(),
            );
        }
        let mut raw = rpc_graph(&decoded);
        if let (Value::List(original), Some(Value::List(targets))) =
            (field(&decoded, "vin"), get_mut(&mut raw, "vin"))
        {
            for (original, target) in original.iter().zip(targets.iter_mut()) {
                if let Value::Dict(_) = original {
                    if let Some(coinbase) = get(original, "coinbase") {
                        if matches!(coinbase, Value::Str(_))
                            && normalized_script_hex(coinbase).is_some()
                        {
                            set(target, "is_coinbase", Value::Bool(true));
                        }
                    }
                    for key in ["scriptSig", "txinwitness"] {
                        if let Some(value) = get(original, key) {
                            set(target, key, value.clone());
                        }
                    }
                }
            }
        }
        raw
    } else if reader.kind == "electrum" {
        let raw_hex = reader.electrum("blockchain.transaction.get", Value::List(vec![s(txid)]))?;
        let Value::Str(raw_hex) = raw_hex else {
            return Err(fail("invalid_observation", "Invalid transaction bytes"));
        };
        let raw_hex = py_text(&Value::Str(raw_hex));
        if raw_hex.chars().count() > 8_000_000 {
            return Err(fail("invalid_observation", "Invalid transaction bytes"));
        }
        if chain == "bitcoin" {
            let (raw, features) = electrum_bitcoin(txid, &raw_hex)?;
            feature_raw = Some(features);
            raw
        } else {
            electrum_liquid(txid, &raw_hex)?
        }
    } else {
        let raw = reader.get(&format!("/tx/{txid}"), false)?;
        if !matches!(raw, Value::Dict(_)) || field(&raw, "txid") != &s(txid) {
            return Err(fail(
                "invalid_observation",
                "Backend returned a different transaction",
            ));
        }
        if let source @ Value::Dict(_) = get(&raw, "status").unwrap_or(&dict::<&str>([])) {
            for key in ["confirmed", "block_height", "block_hash", "block_time"] {
                if let Some(value) = get(source, key) {
                    if matches!(value, Value::Bool(_) | Value::Int(_))
                        || key == "block_hash" && is_txid(value)
                    {
                        set(&mut status, key, value.clone());
                    }
                }
            }
        }
        raw
    };
    let (Value::List(vin), Value::List(vout)) = (field(&raw, "vin"), field(&raw, "vout")) else {
        return Err(fail(
            "invalid_observation",
            "Backend returned an incomplete or oversized transaction",
        ));
    };
    if vin.is_empty() || vout.is_empty() || vin.len() + vout.len() > 20000 {
        return Err(fail(
            "invalid_observation",
            "Backend returned an incomplete or oversized transaction",
        ));
    }
    let mut clean = sanitize(&raw, chain, txid);
    if chain == "bitcoin" {
        let source = feature_raw.as_ref().unwrap_or(&raw);
        let features = crate::analysis_features::extract(
            &to_serde(source),
            "local_acquisition",
            Some(&format!("tx:{txid}")),
            &[],
        );
        set(
            &mut clean,
            crate::analysis_features::PERSISTED_FEATURE_KEY,
            from_serde(&features),
        );
    }
    if list_len(field(&clean, "vin")) != vin.len() || list_len(field(&clean, "vout")) != vout.len()
    {
        return Err(fail(
            "invalid_observation",
            "Backend returned malformed transaction legs",
        ));
    }
    let clean_vin = match get_mut(&mut clean, "vin") {
        Some(Value::List(items)) => items,
        _ => unreachable!("sanitized inputs"),
    };
    // Sequence is public structure; witnesses and preimages are discarded.
    for (original, target) in vin.iter().zip(clean_vin.iter_mut()) {
        if matches!(field(original, "coinbase"), Value::Str(_))
            && normalized_script_hex(field(original, "coinbase")).is_some()
        {
            set(target, "is_coinbase", Value::Bool(true));
        }
        if let Value::Int(sequence) = field(original, "sequence") {
            if *sequence >= BigInt::from(0) && *sequence <= BigInt::from(0xFFFF_FFFFu32) {
                set(target, "sequence", Value::Int(sequence.clone()));
            }
        }
        if !truthy(field(target, "is_coinbase"))
            && !truthy(field(target, "is_pegin"))
            && (input_outpoint(target).is_none() || !matches!(field(target, "vout"), Value::Int(_)))
        {
            return Err(fail(
                "invalid_observation",
                "Backend returned an invalid input outpoint",
            ));
        }
    }
    let outputs = list_len(field(&clean, "vout"));
    let mut seen = BTreeSet::new();
    if let Value::List(items) = field(&clean, "vout") {
        for output in items {
            let number = field(output, "n");
            let script = get(output, "scriptpubkey").unwrap_or(&Value::Null);
            let value = get(output, "value").unwrap_or(&Value::Null);
            let invalid_number = match number {
                Value::Int(n) => {
                    *n < BigInt::from(0)
                        || *n >= BigInt::from(outputs as u64)
                        || !seen.insert(n.clone())
                }
                _ => true,
            };
            let invalid_script = script != &Value::Null
                && (py_text(script).chars().count() > 20000
                    || normalized_script_hex(script).is_none());
            let invalid_value =
                value != &Value::Null && !matches!(value, Value::Int(n) if *n >= BigInt::from(0));
            let unexplained = value == &Value::Null
                && script == &Value::Null
                && field(output, "value_state") != &s("confidential");
            if invalid_number || invalid_script || invalid_value || unexplained {
                return Err(fail(
                    "invalid_observation",
                    "Backend returned an invalid output",
                ));
            }
        }
    }
    Ok((clean, status))
}

fn list_len(value: &Value) -> usize {
    match value {
        Value::List(items) => items.len(),
        _ => 0,
    }
}

fn get_mut<'v>(value: &'v mut Value, key: &str) -> Option<&'v mut Value> {
    match value {
        Value::Dict(pairs) => pairs
            .iter_mut()
            .rev()
            .find(|(k, _)| k == &kassiber_pyjson::Key::from(key))
            .map(|(_, v)| v),
        _ => None,
    }
}

type Claims = Vec<(String, Vec<(String, BigInt)>)>;

fn claim(claimed: &mut Claims, spender: String, point: (String, BigInt)) {
    match claimed.iter_mut().find(|(s, _)| *s == spender) {
        Some((_, points)) => points.push(point),
        None => claimed.push((spender, vec![point])),
    }
}

/// `_Reader.spenders`: claimed spenders by spending txid.
fn spenders(reader: &mut Reader<'_>, txid: &str, raw: &Value) -> Result<Claims> {
    let outputs = list_len(field(raw, "vout"));
    let mut claimed = Claims::new();
    let invalid = || fail("invalid_observation", "Invalid spent-output response");
    if reader.kind == "bitcoinrpc" {
        let refs = (0..outputs)
            .map(|i| dict([("txid", s(txid)), ("vout", int(i as i64))]))
            .collect();
        let rows = reader.rpc(
            "gettxspendingprevout",
            Value::List(vec![
                Value::List(refs),
                dict([("mempool_only", Value::Bool(false))]),
            ]),
        )?;
        let Value::List(rows) = rows else {
            return Err(invalid());
        };
        if rows.len() != outputs
            || rows.iter().enumerate().any(|(i, row)| {
                !matches!(row, Value::Dict(_))
                    || field(row, "txid") != &s(txid)
                    || !matches!(field(row, "vout"), Value::Int(n) if *n == BigInt::from(i as u64))
                    || get(row, "spendingtxid").is_some_and(|v| !is_txid(v))
            })
        {
            return Err(invalid());
        }
        for row in &rows {
            if truthy(field(row, "spendingtxid")) {
                let Value::Int(vout) = field(row, "vout") else {
                    unreachable!()
                };
                claim(
                    &mut claimed,
                    py_text(field(row, "spendingtxid")),
                    (txid.to_owned(), vout.clone()),
                );
            }
        }
        return Ok(claimed);
    }
    let rows = reader.get(&format!("/tx/{txid}/outspends"), false)?;
    let Value::List(rows) = rows else {
        return Err(invalid());
    };
    if rows.len() != outputs
        || rows.iter().any(|row| {
            !matches!(row, Value::Dict(_))
                || !matches!(field(row, "spent"), Value::Bool(_))
                || field(row, "spent") == &Value::Bool(true) && !is_txid(field(row, "txid"))
        })
    {
        return Err(invalid());
    }
    for (number, row) in rows.iter().enumerate() {
        if field(row, "spent") == &Value::Bool(true) {
            claim(
                &mut claimed,
                py_text(field(row, "txid")),
                (txid.to_owned(), BigInt::from(number as u64)),
            );
        }
    }
    Ok(claimed)
}

pub(in super::super) fn apply(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
    runtime: &mut dyn Runtime,
) -> Result<Value> {
    arguments(args, &["plan"], &["plan"])?;
    let supplied = field(args, "plan");
    if !matches!(supplied, Value::Dict(_)) || !matches!(field(supplied, "args"), Value::Dict(_)) {
        return Err(invalid("An acquisition plan is required"));
    }
    let (expected, mut backend, seeds) =
        super::prepare(conn, profile, field(supplied, "args"), sources)?;
    if canonical(&expected)? != canonical(supplied)? {
        return Err(stale(
            "Acquisition plan changed; review a new plan before fetching",
        ));
    }
    let value = field(&expected, "args").clone();
    let safe = field(&expected, "backend").clone();
    // The exact backend row and seeds bound to the recomputed plan; the
    // explicit grant below is minted only at this verified action boundary.
    let timeout = backend_timeout(&backend)?.min(8);
    set(&mut backend, "timeout", int(timeout));
    let chain = py_text(field(&value, "chain"));
    let (depth_limit, maximum) = (
        small(field(&value, "depth")),
        small(field(&value, "max_transactions")),
    );
    let direction = py_text(field(&value, "direction"));
    let mut reader = Reader::new(backend.clone(), &*runtime)?;
    reader.budget = Budget::new(small(field(field(&expected, "effects"), "max_requests")));
    let outcome = fetch(
        &mut reader,
        &value,
        &seeds,
        &chain,
        &direction,
        depth_limit,
        maximum,
    );
    // Revoke the request-local grant before any local write.
    reader.revoke();
    let (observations, frontier, count, bytes) = outcome?;
    let network = normalize_network(&chain, &py_text(field(&value, "network"))).map_err(
        |error| match error {
            crate::descriptors::Error::Value(message) => AppError::new("internal_error", message),
            crate::descriptors::Error::App(error) => error,
        },
    )?;
    // Revalidate book and backend before the atomic save.
    atomic(conn, || {
        if field(&super::plan(conn, profile, &value, sources)?, "plan_id")
            != field(&expected, "plan_id")
        {
            return Err(stale(
                "Local inputs changed during acquisition; observations were not saved",
            ));
        }
        let observed = sources.now();
        for (txid, (raw, status)) in &observations {
            conn.execute(
                "INSERT INTO chain_analysis_observations VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(profile_id,chain,network,txid) DO UPDATE SET payload_json=excluded.payload_json,status_json=excluded.status_json,source_name=excluded.source_name,observed_at=excluded.observed_at",
                params![profile, chain, network, txid, canonical(raw)?, canonical(status)?, py_text(field(&backend, "name")), observed],
            )
            .map_err(database_error)?;
        }
        Ok(())
    })?;
    let mut ids: Vec<&String> = observations.iter().map(|(txid, _)| txid).collect();
    ids.sort();
    let frontier: BTreeSet<(String, String)> = frontier.into_iter().collect();
    let snapshot = super::super::watches::revision_for(conn, profile, sources)?;
    Ok(dict([
        ("acquired_count", int(observations.len() as i64)),
        ("request_count", int(count)),
        ("response_bytes", int(bytes)),
        (
            "transaction_ids",
            Value::List(ids.into_iter().map(|t| s(t.as_str())).collect()),
        ),
        (
            "frontier",
            Value::List(
                frontier
                    .iter()
                    .map(|(txid, reason)| {
                        dict([("txid", s(txid.as_str())), ("reason", s(reason.as_str()))])
                    })
                    .collect(),
            ),
        ),
        ("complete", Value::Bool(frontier.is_empty())),
        ("backend", safe),
        ("snapshot_id", field(&snapshot, "snapshot_id").clone()),
    ]))
}

type Observations = Vec<(String, (Value, Value))>;
/// Observations, frontier, request count and response bytes.
type Fetched = (Observations, Vec<(String, String)>, i64, i64);

fn fetch(
    reader: &mut Reader<'_>,
    value: &Value,
    seeds: &[String],
    chain: &str,
    direction: &str,
    depth_limit: i64,
    maximum: i64,
) -> Result<Fetched> {
    if reader.kind == "electrum" {
        // The server.version handshake is a real protocol request.
        reader.budget.request(reader.runtime())?;
        reader.connect_electrum()?;
    }
    if !py_eq(&reader.genesis()?, field(value, "genesis_hash")) {
        return Err(fail(
            "backend_network_mismatch",
            "Backend genesis does not match the selected network",
        ));
    }
    let mut forward = reader.kind != "electrum";
    if reader.kind == "bitcoinrpc" && direction != "backward" {
        forward = match reader.rpc("getindexinfo", Value::List(vec![s("txospenderindex")])) {
            Ok(indexes) => {
                matches!(indexes, Value::Dict(_))
                    && field(field(&indexes, "txospenderindex"), "synced") == &Value::Bool(true)
            }
            Err(_) => false,
        };
    }
    let mut queue: VecDeque<(String, i64)> = seeds.iter().map(|seed| (seed.clone(), 0)).collect();
    let mut seen = BTreeSet::new();
    let mut observations: Observations = Vec::new();
    let mut frontier = Vec::new();
    let mut spend_claims = Claims::new();
    let stop = |frontier: &mut Vec<(String, String)>, txid: &str, reason: &str| {
        frontier.push((txid.to_owned(), reason.to_owned()));
    };
    while let Some((txid, depth)) = queue.pop_front() {
        if seen.contains(&txid) {
            continue;
        }
        if seen.len() as i64 >= maximum {
            stop(&mut frontier, &txid, "transaction_limit");
            continue;
        }
        seen.insert(txid.clone());
        let (raw, mut status) = match transaction(reader, &txid, chain) {
            Ok(found) => found,
            Err(error) => {
                let reason = if error.code == "acquisition_budget" {
                    "acquisition_budget"
                } else if reader.budget.count >= reader.budget.maximum || reader.budget.expired() {
                    "budget_or_history_unavailable"
                } else {
                    "transaction_unavailable"
                };
                stop(&mut frontier, &txid, reason);
                continue;
            }
        };
        let mut neighbors = BTreeSet::new();
        if direction != "forward" {
            if let Value::List(vin) = field(&raw, "vin") {
                for row in vin {
                    if let Some(prev) = txid_text(field(row, "txid")) {
                        if !truthy(field(row, "is_coinbase")) && !truthy(field(row, "is_pegin")) {
                            neighbors.insert(prev);
                        }
                    }
                }
            }
        }
        if direction != "backward" {
            if !forward {
                stop(&mut frontier, &txid, "historical_spender_index_unavailable");
            } else {
                match spenders(reader, &txid, &raw) {
                    Ok(claimed) => {
                        for (spender, points) in claimed {
                            neighbors.insert(spender.clone());
                            for point in points {
                                claim(&mut spend_claims, spender.clone(), point);
                            }
                        }
                        set(&mut status, "spenders_checked", Value::Bool(true));
                    }
                    Err(_) => stop(&mut frontier, &txid, "spender_history_unavailable"),
                }
            }
        }
        observations.push((txid.clone(), (raw, status)));
        for neighbor in neighbors {
            if seen.contains(&neighbor) {
                continue;
            }
            if depth >= depth_limit {
                stop(&mut frontier, &neighbor, "depth_limit");
            } else {
                queue.push_back((neighbor, depth + 1));
            }
        }
    }
    for (spender, points) in &spend_claims {
        let Some(index) = observations.iter().position(|(txid, _)| txid == spender) else {
            continue; // Already represented by a history/depth/budget frontier.
        };
        let spent: BTreeSet<(String, BigInt)> = match field(&observations[index].1 .0, "vin") {
            Value::List(vin) => vin
                .iter()
                .filter(|item| !truthy(field(item, "is_pegin")))
                .filter_map(input_outpoint)
                .collect(),
            _ => BTreeSet::new(),
        };
        let unique: BTreeSet<&(String, BigInt)> = points.iter().collect();
        for (source, number) in unique {
            if spent.contains(&(source.clone(), number.clone())) {
                continue;
            }
            stop(&mut frontier, source, "spender_claim_not_corroborated");
            if let Some((_, (_, status))) = observations.iter_mut().find(|(txid, _)| txid == source)
            {
                set(status, "spenders_checked", Value::Bool(false));
            }
        }
    }
    Ok((
        observations,
        frontier,
        reader.budget.count,
        reader.budget.bytes_read,
    ))
}
