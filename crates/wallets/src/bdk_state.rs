//! Explicit conversion between Kassiber's private version-one JSON and BDK.
//! Never give BDK a database path, file store, or independent persistence.
use bdk_chain::{BlockId, ConfirmationBlockTime, DescriptorId};
use bdk_wallet::ChangeSet;
use bitcoin::{
    consensus, hashes::Hash, hex::FromHex, Amount, BlockHash, Network, OutPoint, ScriptBuf, TxOut,
    Txid,
};
use kassiber_envelope::AppError;
use serde_json::{json, Value};
use std::str::FromStr;

type Result<T> = std::result::Result<T, AppError>;
pub(crate) fn invalid() -> AppError {
    AppError::new(
        "observer_state_rebuild_required",
        "Stored BDK observer state cannot be decoded",
    )
    .with_hint("Run a full wallet refresh to rebuild derived BDK observer state.")
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    Vec::from_hex(v.as_str().ok_or_else(invalid)?).map_err(|_| invalid())
}
fn txid(v: &Value) -> Result<Txid> {
    Txid::from_slice(&bytes(v)?).map_err(|_| invalid())
}
fn block_hash(v: &Value) -> Result<BlockHash> {
    BlockHash::from_slice(&bytes(v)?).map_err(|_| invalid())
}
fn outpoint(v: &Value) -> Result<OutPoint> {
    Ok(OutPoint {
        txid: txid(&v["txid"])?,
        vout: v["vout"].as_u64().ok_or_else(invalid)? as u32,
    })
}
fn list(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().ok_or_else(invalid)
}
fn serialized_hash(v: impl AsRef<[u8]>) -> String {
    v.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn network(name: &str) -> Result<Network> {
    match name {
        "BITCOIN" | "main" => Ok(Network::Bitcoin),
        "TESTNET" | "test" | "testnet" => Ok(Network::Testnet),
        "TESTNET4" | "testnet4" => Ok(Network::Testnet4),
        "SIGNET" | "signet" => Ok(Network::Signet),
        "REGTEST" | "regtest" => Ok(Network::Regtest),
        _ => Err(invalid()),
    }
}

pub(crate) fn decode(payload: &Value) -> Result<ChangeSet> {
    let p = crate::observer_state::normalize_changeset(payload)?;
    let descriptor = |key| {
        p[key]
            .as_str()
            .map(FromStr::from_str)
            .transpose()
            .map_err(|_| invalid())
    };
    let mut c = ChangeSet {
        descriptor: descriptor("descriptor")?,
        change_descriptor: descriptor("change_descriptor")?,
        network: p["network"].as_str().map(network).transpose()?,
        ..Default::default()
    };
    for item in list(&p["local_chain"])? {
        c.local_chain.blocks.insert(
            item["height"].as_u64().ok_or_else(invalid)? as u32,
            if item["hash"].is_null() {
                None
            } else {
                Some(block_hash(&item["hash"])?)
            },
        );
    }
    let g = &p["tx_graph"];
    for tx in list(&g["txs"])? {
        c.tx_graph.txs.insert(
            consensus::deserialize::<bitcoin::Transaction>(&bytes(tx)?)
                .map_err(|_| invalid())?
                .into(),
        );
    }
    for item in list(&g["txouts"])? {
        c.tx_graph.txouts.insert(
            outpoint(item)?,
            TxOut {
                value: Amount::from_sat(item["value_sats"].as_u64().ok_or_else(invalid)?),
                script_pubkey: ScriptBuf::from_bytes(bytes(&item["script_pubkey"])?),
            },
        );
    }
    for item in list(&g["anchors"])? {
        c.tx_graph.anchors.insert((
            ConfirmationBlockTime {
                block_id: BlockId {
                    height: item["block_height"].as_u64().ok_or_else(invalid)? as u32,
                    hash: block_hash(&item["block_hash"])?,
                },
                confirmation_time: item["confirmation_time"].as_u64().ok_or_else(invalid)?,
            },
            txid(&item["txid"])?,
        ));
    }
    for (key, dest) in [
        ("last_seen", &mut c.tx_graph.last_seen),
        ("first_seen", &mut c.tx_graph.first_seen),
        ("last_evicted", &mut c.tx_graph.last_evicted),
    ] {
        for item in list(&g[key])? {
            dest.insert(
                txid(&item["txid"])?,
                item["timestamp"].as_u64().ok_or_else(invalid)?,
            );
        }
    }
    for item in list(&p["indexer"])? {
        c.indexer.last_revealed.insert(
            DescriptorId::from_slice(&bytes(&item["descriptor_id"])?).map_err(|_| invalid())?,
            item["last_revealed"].as_u64().ok_or_else(invalid)? as u32,
        );
    }
    for item in list(&p["locked_outpoints"])? {
        c.locked_outpoints.outpoints.insert(
            outpoint(item)?,
            item["locked"].as_bool().ok_or_else(invalid)?,
        );
    }
    Ok(c)
}

pub(crate) fn encode(c: &ChangeSet) -> Value {
    let mut txs: Vec<_> = c
        .tx_graph
        .txs
        .iter()
        .map(|tx| consensus::encode::serialize_hex(tx.as_ref()))
        .collect();
    txs.sort();
    let op = |o: &OutPoint| json!({"txid":serialized_hash(o.txid.to_byte_array()), "vout":o.vout});
    let mut txouts: Vec<_> = c
        .tx_graph
        .txouts
        .iter()
        .map(|(o, t)| {
            let mut v = op(o);
            v["value_sats"] = json!(t.value.to_sat());
            v["script_pubkey"] = json!(t.script_pubkey.to_hex_string());
            v
        })
        .collect();
    txouts.sort_by_key(|v| {
        (
            v["txid"].as_str().unwrap().to_owned(),
            v["vout"].as_u64().unwrap(),
        )
    });
    let mut anchors: Vec<_> = c.tx_graph.anchors.iter().map(|(a,t)| json!({"txid":serialized_hash(t.to_byte_array()), "block_height":a.block_id.height, "block_hash":serialized_hash(a.block_id.hash.to_byte_array()), "confirmation_time":a.confirmation_time})).collect();
    anchors.sort_by_key(|v| {
        (
            v["txid"].as_str().unwrap().to_owned(),
            v["block_height"].as_u64().unwrap(),
            v["block_hash"].as_str().unwrap().to_owned(),
        )
    });
    let times = |values: &std::collections::BTreeMap<Txid, u64>| {
        let mut v: Vec<_> = values
            .iter()
            .map(|(t, s)| json!({"txid":serialized_hash(t.to_byte_array()),"timestamp":s}))
            .collect();
        v.sort_by_key(|v| v["txid"].as_str().unwrap().to_owned());
        v
    };
    let mut indices: Vec<_> = c
        .indexer
        .last_revealed
        .iter()
        .map(|(d, i)| json!({"descriptor_id":serialized_hash(d.to_byte_array()),"last_revealed":i}))
        .collect();
    indices.sort_by_key(|v| v["descriptor_id"].as_str().unwrap().to_owned());
    let mut locked: Vec<_> = c
        .locked_outpoints
        .outpoints
        .iter()
        .map(|(o, b)| {
            let mut v = op(o);
            v["locked"] = json!(b);
            v
        })
        .collect();
    locked.sort_by_key(|v| {
        (
            v["txid"].as_str().unwrap().to_owned(),
            v["vout"].as_u64().unwrap(),
        )
    });
    let name = c.network.map(|n| match n {
        Network::Bitcoin => "BITCOIN",
        Network::Testnet => "TESTNET",
        Network::Testnet4 => "TESTNET4",
        Network::Signet => "SIGNET",
        Network::Regtest => "REGTEST",
    });
    json!({"schema_version":1,"descriptor":c.descriptor.as_ref().map(ToString::to_string),"change_descriptor":c.change_descriptor.as_ref().map(ToString::to_string),"network":name,
        "local_chain":c.local_chain.blocks.iter().map(|(h,b)|json!({"height":h,"hash":b.map(|b|serialized_hash(b.to_byte_array()))})).collect::<Vec<_>>(),
        "tx_graph":{"txs":txs,"txouts":txouts,"anchors":anchors,"last_seen":times(&c.tx_graph.last_seen),"first_seen":times(&c.tx_graph.first_seen),"last_evicted":times(&c.tx_graph.last_evicted)},
        "indexer":indices,"locked_outpoints":locked})
}
