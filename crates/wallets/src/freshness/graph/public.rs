//! The network half of the desktop graph's explicit public reference lookup
//! (`_fetch_reference_graph_from_backend`): one backend's transaction, and
//! for Electrum its parents, decoded into Python's graph shapes; Bitcoin
//! Core's verbose replies as they come. The caller chose the backend and owns
//! decoding Core's replies, sanitizing, caching and warnings; every request
//! runs under a grant for the user's explicit lookup.
use super::lookup::selected;
use crate::publication::row::Result;
use bitcoin::{consensus::deserialize, hex::FromHex, Transaction};
use kassiber_chain::{Backend, CoreClient, ElectrumClient, HttpClient};
use kassiber_envelope::{dict, s, AppError};
use kassiber_net::{Consent, Gate};
use kassiber_pyjson::Value as Py;
use serde_json::Value as Json;
use std::time::Instant;

fn invalid_observation() -> AppError {
    AppError::new(
        "invalid_observation",
        "Electrum returned different transaction bytes",
    )
}

fn bytes(raw: &Json) -> Result<Vec<u8>> {
    Vec::from_hex(raw.as_str().unwrap_or(""))
        .map_err(|_| AppError::new("protocol_error", "Invalid public transaction reference"))
}

fn int(value: impl Into<kassiber_pyjson::BigInt>) -> Py {
    Py::Int(value.into())
}

fn hex(raw: &[u8]) -> String {
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

/// `_bitcoin_electrum_decoded_to_graph_raw(txid, _decode_verified_bitcoin_transaction(raw_hex, txid))`.
fn bitcoin_graph(raw: &Json, id: &str) -> Result<Py> {
    let payload = bytes(raw)?;
    let tx: Transaction = deserialize(&payload)
        .map_err(|_| AppError::new("protocol_error", "Invalid public transaction reference"))?;
    if !tx.compute_txid().to_string().eq_ignore_ascii_case(id) {
        return Err(invalid_observation());
    }
    let vin = tx
        .input
        .iter()
        .map(|input| {
            if input.previous_output.is_null() {
                dict([("is_coinbase", Py::Bool(true))])
            } else {
                dict([
                    ("txid", s(input.previous_output.txid.to_string().as_str())),
                    ("vout", int(input.previous_output.vout)),
                ])
            }
        })
        .collect();
    let vout = tx
        .output
        .iter()
        .enumerate()
        .map(|(n, output)| {
            let mut entry = vec![("n", int(n as u64))];
            let script = output.script_pubkey.to_hex_string();
            if !script.is_empty() {
                entry.push(("scriptpubkey", s(script.as_str())));
            }
            entry.push(("value", int(output.value.to_sat())));
            dict(entry)
        })
        .collect();
    Ok(dict([
        ("txid", s(id.to_ascii_lowercase().as_str())),
        ("version", int(tx.version.0)),
        ("locktime", int(tx.lock_time.to_consensus_u32())),
        ("vin", Py::List(vin)),
        ("vout", Py::List(vout)),
        ("size", int(payload.len() as u64)),
        ("vsize", int(tx.vsize() as u64)),
        ("weight", int(tx.weight().to_wu())),
    ]))
}

/// `_liquid_electrum_decoded_to_graph_raw(txid, decode_liquid_transaction(raw_hex))`
/// after the txid check.
fn liquid_graph(raw: &Json, id: &str) -> Result<Py> {
    use elements_miniscript::elements::{self, confidential};
    let payload = bytes(raw)?;
    let tx: elements::Transaction = elements::encode::deserialize(&payload)
        .map_err(|_| AppError::new("protocol_error", "Invalid public transaction reference"))?;
    if !tx.txid().to_string().eq_ignore_ascii_case(id) {
        return Err(invalid_observation());
    }
    let vin = tx
        .input
        .iter()
        .map(|input| {
            dict([
                ("txid", s(input.previous_output.txid.to_string().as_str())),
                ("vout", int(input.previous_output.vout)),
                ("prevout", dict([("value_state", s("confidential"))])),
            ])
        })
        .collect();
    let vout = tx
        .output
        .iter()
        .enumerate()
        .map(|(n, output)| {
            let script = hex(output.script_pubkey.as_bytes());
            let mut entry = vec![("n", int(n as u64))];
            match (&output.value, &output.nonce) {
                _ if !script.is_empty() => {
                    entry.push(("scriptpubkey", s(script.as_str())));
                    entry.push(("value_state", s("confidential")));
                }
                (confidential::Value::Explicit(value), confidential::Nonce::Null) => {
                    entry.push(("scriptpubkey_type", s("fee")));
                    entry.push(("value", int(*value)));
                }
                _ => entry.push(("value_state", s("confidential"))),
            }
            dict(entry)
        })
        .collect();
    Ok(dict([
        ("txid", s(id.to_ascii_lowercase().as_str())),
        ("version", int(tx.version)),
        ("locktime", int(tx.lock_time.to_consensus_u32())),
        ("vin", Py::List(vin)),
        ("vout", Py::List(vout)),
        ("size", int(payload.len() as u64)),
    ]))
}

/// `now_iso()`: the current UTC second.
pub fn now_iso() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    kassiber_chain::format_unix_time(seconds).unwrap_or_default()
}

/// One explicitly requested public lookup's transport: the user asked for
/// this graph's references, so each selected backend gets an explicit grant.
pub struct Network<'a> {
    pub gate: &'a Gate,
    pub version: &'a str,
}

impl Network<'_> {
    /// `_fetch_graph_esplora_transaction(backend, txid)`.
    pub fn esplora(&self, backend: &Json, txid: &str, deadline: Instant) -> Result<Py> {
        let (transport, _) = selected(backend)?;
        let consent = Consent::explicit_trigger(transport.request());
        let client = HttpClient::new(transport, self.gate.clone(), self.version);
        let raw = client.get_json(&format!("tx/{txid}"), Some(&consent), deadline)?;
        Ok(kassiber_pytext::loads(&raw.to_string()).expect("JSON value"))
    }

    /// A Bitcoin Core client for `_fetch_bitcoinrpc_transaction_graph`, at
    /// `_graph_lookup_timeout`: the configured timeout, at most five seconds.
    pub fn core(&self, backend: &Json, deadline: Instant) -> Result<Core> {
        let mut backend = backend.clone();
        let configured = crate::scan::integer(&backend["timeout"])
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n > 0)
            .unwrap_or(5)
            .min(5);
        backend["timeout"] = serde_json::json!(configured);
        let transport = Backend::new(&backend, false)?;
        let consent = Consent::explicit_trigger(transport.request());
        Ok(Core {
            client: CoreClient::new(transport, self.gate.clone(), self.version),
            consent,
            deadline,
        })
    }

    /// An Electrum connection for `_fetch_*_electrum_graph_raw`.
    pub fn electrum(&self, backend: &Json, deadline: Instant) -> Result<Electrum> {
        let (transport, _) = selected(backend)?;
        let consent = Consent::explicit_trigger(transport.request());
        Ok(Electrum {
            client: ElectrumClient::connect(transport, self.gate, Some(consent), deadline)?,
            deadline,
        })
    }
}

pub struct Electrum {
    client: ElectrumClient,
    deadline: Instant,
}

pub struct Core {
    client: CoreClient,
    consent: Consent,
    deadline: Instant,
}

impl Core {
    /// `bitcoinrpc_call(backend, "getrawtransaction", [txid, True])`: the
    /// verbose transaction as Core returned it.
    pub fn transaction(&self, txid: &str) -> Result<Py> {
        let raw = self.client.call(
            "getrawtransaction",
            Some(&serde_json::json!([txid, true])),
            None,
            Some(&self.consent),
            self.deadline,
        )?;
        Ok(kassiber_pytext::loads(&raw.to_string()).expect("JSON value"))
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

impl Electrum {
    /// The current Bitcoin transaction, verified against its txid.
    pub fn bitcoin(&mut self, txid: &str) -> Result<Py> {
        let raw = self.client.call(
            "blockchain.transaction.get",
            Some(&serde_json::json!([txid])),
            self.deadline,
        )?;
        bitcoin_graph(&raw, txid)
    }

    /// `_electrum_missing_prevout_graphs`: parents in batches of the
    /// backend's batch size, each verified.
    pub fn parents(&mut self, txids: &[String]) -> Result<Vec<(String, Py)>> {
        let mut fetched = Vec::new();
        for ids in txids.chunks(self.client.batch_size().max(1)) {
            let requests: Vec<_> = ids
                .iter()
                .map(|id| ("blockchain.transaction.get".into(), serde_json::json!([id])))
                .collect();
            let replies = self.client.batch_call(&requests, self.deadline)?;
            for (id, raw) in ids.iter().zip(&replies) {
                fetched.push((id.clone(), bitcoin_graph(raw, id)?));
            }
        }
        Ok(fetched)
    }

    /// The current Liquid transaction, verified against its txid.
    pub fn liquid(&mut self, txid: &str) -> Result<Py> {
        let raw = self.client.call(
            "blockchain.transaction.get",
            Some(&serde_json::json!([txid])),
            self.deadline,
        )?;
        liquid_graph(&raw, txid)
    }
}
