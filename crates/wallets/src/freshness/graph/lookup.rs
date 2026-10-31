use super::references::{self, cached, complete, sanitize, store, string, txid};
use crate::{bitcoin_records::value_sats, publication::row::Result};
use bitcoin::{consensus::deserialize, hex::FromHex, Transaction};
use kassiber_chain::{Backend, ElectrumClient, HttpClient};
use kassiber_envelope::AppError;
use kassiber_net::{Consent, Gate};
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

pub(super) struct Lookup<'a> {
    pub conn: &'a Connection,
    pub backend: Backend,
    pub electrum: bool,
    pub gate: &'a Gate,
    pub consent: Consent,
    pub check: &'a dyn Fn() -> Result<()>,
    pub version: &'a str,
    pub network: String,
    pub remaining: usize,
    pub requested: usize,
    pub refused: bool,
}

fn invalid() -> AppError {
    AppError::new(
        "protocol_error",
        "Invalid public Bitcoin transaction reference",
    )
}

fn decode(raw: &Value, id: &str) -> Result<Value> {
    let bytes = Vec::from_hex(raw.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?;
    let tx: Transaction = deserialize(&bytes).map_err(|_| invalid())?;
    if !tx.compute_txid().to_string().eq_ignore_ascii_case(id) {
        return Err(invalid());
    }
    let vin: Vec<_> = tx.input.iter().map(|input| {
        if input.previous_output.is_null() {
            json!({"is_coinbase":true})
        } else {
            json!({"txid":input.previous_output.txid.to_string(),"vout":input.previous_output.vout})
        }
    }).collect();
    let vout: Vec<_> = tx.output.iter().enumerate().map(|(n,output)|
        json!({"n":n,"scriptpubkey":output.script_pubkey.to_hex_string(),"value":output.value.to_sat()})
    ).collect();
    Ok(
        json!({"txid":id,"version":tx.version.0,"locktime":tx.lock_time.to_consensus_u32(),
        "size":bytes.len(),"vsize":tx.vsize(),"weight":tx.weight().to_wu(),"vin":vin,"vout":vout}),
    )
}

impl Lookup<'_> {
    pub fn fetch(&mut self, id: &str, now: &str) -> Result<bool> {
        (self.check)()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let raw = if self.electrum {
            let mut client = ElectrumClient::connect(
                self.backend.clone(),
                self.gate,
                Some(self.consent.clone()),
                deadline,
            )?;
            (self.check)()?;
            let raw = client.call("blockchain.transaction.get", Some(&json!([id])), deadline)?;
            (self.check)()?;
            let mut raw = decode(&raw, id)?;
            self.attach(&mut client, &mut raw, now, deadline)?;
            raw
        } else {
            let client = HttpClient::new(self.backend.clone(), self.gate.clone(), self.version);
            client.get_json_checked(
                &format!("tx/{id}"),
                Some(&self.consent),
                deadline,
                self.check,
            )?
        };
        (self.check)()?;
        if !raw.is_object() {
            return Ok(false);
        }
        let raw = sanitize(&raw, id);
        if string(&raw["txid"]).is_some_and(|v| !v.eq_ignore_ascii_case(id)) || !complete(&raw) {
            return Ok(false);
        }
        store(
            self.conn,
            "bitcoin",
            &self.network,
            id,
            &raw,
            now,
            self.check,
        )?;
        Ok(true)
    }

    fn attach(
        &mut self,
        client: &mut ElectrumClient,
        raw: &mut Value,
        now: &str,
        deadline: Instant,
    ) -> Result<()> {
        let mut missing = Vec::new();
        let mut seen = BTreeSet::new();
        let mut previous = BTreeMap::new();
        for (index, entry) in raw["vin"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .enumerate()
        {
            if index % 64 == 0 {
                (self.check)()?;
                if Instant::now() >= deadline {
                    return Err(kassiber_net::Error::InvalidTimeout.app_error());
                }
            }
            let Some(id) = txid(&entry["txid"]) else {
                continue;
            };
            if entry["vout"].as_u64().is_none()
                || value_sats(&entry["prevout"]).ok().flatten().is_some()
                || !seen.insert(id.clone())
            {
                continue;
            }
            if let Some(cached) = cached(self.conn, "bitcoin", &self.network, &id)? {
                previous.insert(id, cached);
            } else {
                missing.push(id);
                if missing.len() > 250 {
                    return Ok(());
                }
            }
        }
        if missing.len() > self.remaining {
            self.refused = true;
            return Ok(());
        }
        self.remaining -= missing.len();
        self.requested += missing.len();
        let mut fetched = Vec::new();
        for ids in missing.chunks(client.batch_size()) {
            (self.check)()?;
            let requests: Vec<_> = ids
                .iter()
                .map(|id| ("blockchain.transaction.get".into(), json!([id])))
                .collect();
            let replies = client.batch_call(&requests, deadline)?;
            (self.check)()?;
            for (id, raw) in ids.iter().zip(&replies) {
                fetched.push((id.clone(), decode(raw, id)?));
            }
        }
        for (id, raw) in fetched {
            previous.insert(
                id.clone(),
                store(
                    self.conn,
                    "bitcoin",
                    &self.network,
                    &id,
                    &raw,
                    now,
                    self.check,
                )?,
            );
        }
        for entry in raw["vin"].as_array_mut().ok_or_else(invalid)? {
            let (Some(id), Some(n)) = (txid(&entry["txid"]), entry["vout"].as_u64()) else {
                continue;
            };
            if let Some(output) = previous
                .get(&id)
                .and_then(|v| v["vout"].as_array())
                .and_then(|v| v.iter().find(|v| v["n"].as_u64() == Some(n)))
            {
                let mut output = output.clone();
                if let Some(output) = output.as_object_mut() {
                    output.retain(|key, _| {
                        [
                            "scriptpubkey",
                            "scriptpubkey_type",
                            "scriptpubkey_address",
                            "value",
                            "value_state",
                        ]
                        .contains(&key.as_str())
                    });
                }
                entry["prevout"] = output;
            }
        }
        Ok(())
    }
}

pub(super) fn selected(backend: &Value) -> Result<(Backend, bool)> {
    let electrum = backend["kind"] == "electrum";
    if !electrum && backend["kind"] != "esplora" && backend["kind"] != "mempool" {
        return Err(AppError::new(
            "not_implemented",
            "Native graph lookup requires an Esplora or Electrum backend",
        ));
    }
    let mut backend = backend.clone();
    if !electrum {
        let mut url = kassiber_net::HttpUrl::parse(backend["url"].as_str().unwrap_or(""))
            .map_err(|_| invalid())?;
        let path = url.path().trim_end_matches('/');
        let path = if path.to_ascii_lowercase().ends_with("/api") {
            path.to_owned()
        } else {
            format!("{path}/api")
        };
        url.set_path(&path);
        url.set_query(None);
        url.set_fragment(None);
        backend["url"] = json!(url.to_string());
    }
    let configured = crate::scan::integer(&backend["timeout"])
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| *n > 0)
        .unwrap_or(5)
        .min(5);
    backend["timeout"] = json!(configured);
    Ok((Backend::new(&backend, electrum)?, electrum))
}

pub(super) fn locally_complete(
    conn: &Connection,
    row: &Value,
    raw: &Value,
    profile: &str,
    network: &str,
    id: &str,
) -> Result<bool> {
    let binding = kassiber_setup::networks::resolve(conn, profile)?;
    if !kassiber_setup::networks::observation_matches_binding(&binding, &references::py(row), false)
    {
        return Ok(true);
    }
    match references::local(conn, profile, "bitcoin", network, id, raw)? {
        references::Local::Conflict => return Ok(true),
        references::Local::Payload(raw) if complete(&raw) => return Ok(true),
        _ => {}
    }
    Ok(cached(conn, "bitcoin", network, id)?
        .as_ref()
        .is_some_and(complete))
}
