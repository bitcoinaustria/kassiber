//! LWK's blocking `BlockchainBackend` over Kassiber's own Electrum client.
//! LWK never opens a socket: every request below goes through `Rpc`, which
//! the product implements over `kassiber_chain::ElectrumClient` and checks
//! cancellation, consent and the deadline around each call.
use super::Result;
use crate::scan::Control;
use kassiber_chain::ElectrumClient;
use kassiber_envelope::AppError;
use lwk_wollet::{
    clients::blocking::BlockchainBackend,
    elements::{
        encode::deserialize, hex::FromHex, BlockHash, BlockHeader, Script, Transaction, Txid,
    },
    Error, History,
};
use serde_json::{json, Value};
use std::{cell::RefCell, collections::HashMap, str::FromStr, time::Instant};

/// The Electrum methods LWK 0.18.0's own client issues, one at a time or as
/// one batch per call site.
pub trait Rpc {
    fn call(&mut self, method: &str, params: Value) -> Result<Value>;
    fn batch(&mut self, method: &str, params: Vec<Value>) -> Result<Vec<Value>> {
        params.into_iter().map(|p| self.call(method, p)).collect()
    }
}

pub(crate) struct Wire<'a> {
    pub client: ElectrumClient,
    pub deadline: Instant,
    pub cancel: &'a dyn Control,
}
impl Rpc for Wire<'_> {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.cancel.check(self.deadline)?;
        let result = self.client.call(method, Some(&params), self.deadline)?;
        self.cancel.check(self.deadline)?;
        Ok(result)
    }
    fn batch(&mut self, method: &str, params: Vec<Value>) -> Result<Vec<Value>> {
        let mut result = vec![];
        for chunk in params.chunks(self.client.batch_size()) {
            self.cancel.check(self.deadline)?;
            result.extend(
                self.client.batch_call(
                    &chunk
                        .iter()
                        .map(|p| (method.into(), p.clone()))
                        .collect::<Vec<_>>(),
                    self.deadline,
                )?,
            );
        }
        self.cancel.check(self.deadline)?;
        Ok(result)
    }
}

fn invalid() -> AppError {
    AppError::new("protocol_error", "Invalid LWK Liquid observation response")
}

/// The first Kassiber error is kept so a cancellation, revocation or offline
/// switch reaches the caller unchanged instead of as an LWK string.
pub(crate) struct Backend<'a, R: Rpc> {
    rpc: RefCell<&'a mut R>,
    failure: RefCell<Option<AppError>>,
}
impl<'a, R: Rpc> Backend<'a, R> {
    pub fn new(rpc: &'a mut R) -> Self {
        Self {
            rpc: RefCell::new(rpc),
            failure: RefCell::new(None),
        }
    }
    pub fn take_failure(&self) -> Option<AppError> {
        self.failure.borrow_mut().take()
    }
    fn fail(&self, error: AppError) -> Error {
        let mut failure = self.failure.borrow_mut();
        if failure.is_none() {
            *failure = Some(error);
        }
        Error::Generic("Kassiber transport refused or failed".into())
    }
    fn guarded<T>(&self, run: impl FnOnce(&mut R) -> Result<T>) -> std::result::Result<T, Error> {
        // After one failure no further request is sent in this scan.
        if self.failure.borrow().is_some() {
            return Err(Error::Generic(
                "Kassiber transport refused or failed".into(),
            ));
        }
        let mut rpc = self.rpc.borrow_mut();
        run(&mut rpc).map_err(|e| self.fail(e))
    }
    /// `blockchain.headers.subscribe`, as LWK's client does on construction
    /// and whenever no header notification is queued.
    pub fn subscribe(&self) -> std::result::Result<BlockHeader, Error> {
        self.guarded(|rpc| {
            let tip = rpc.call("blockchain.headers.subscribe", json!([]))?;
            decode::<BlockHeader>(&tip["hex"])
        })
    }
}

fn decode<T: lwk_wollet::elements::encode::Decodable>(value: &Value) -> Result<T> {
    let bytes = Vec::<u8>::from_hex(value.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?;
    deserialize(&bytes).map_err(|_| invalid())
}

fn script_hash(script: &Script) -> Result<String> {
    crate::scan::script_hash(&lwk_wollet::elements::hex::ToHex::to_hex(script.as_bytes()))
}

impl<R: Rpc> BlockchainBackend for Backend<'_, R> {
    fn tip(&mut self) -> std::result::Result<BlockHeader, Error> {
        // Kassiber's client never queues header notifications, so LWK's
        // client would always take its re-subscribe branch here.
        self.subscribe()
    }

    fn broadcast(&self, _tx: &Transaction) -> std::result::Result<Txid, Error> {
        // Watch-only boundary: no transaction ever leaves through this route.
        Err(self.fail(AppError::new(
            "observer_capability_unsupported",
            "Kassiber's Liquid observer never broadcasts transactions",
        )))
    }

    fn get_transactions(&self, txids: &[Txid]) -> std::result::Result<Vec<Transaction>, Error> {
        self.guarded(|rpc| {
            let raws = rpc.batch(
                "blockchain.transaction.get",
                txids.iter().map(|id| json!([id.to_string()])).collect(),
            )?;
            if raws.len() != txids.len() {
                return Err(invalid());
            }
            raws.iter()
                .zip(txids)
                .map(|(raw, id)| {
                    let tx: Transaction = decode(raw)?;
                    // A server cannot substitute another transaction's bytes.
                    if tx.txid() != *id {
                        return Err(invalid());
                    }
                    Ok(tx)
                })
                .collect()
        })
    }

    fn get_headers(
        &self,
        heights: &[u32],
        _: &HashMap<u32, BlockHash>,
    ) -> std::result::Result<Vec<BlockHeader>, Error> {
        self.guarded(|rpc| {
            let raws = rpc.batch(
                "blockchain.block.header",
                heights.iter().map(|h| json!([h])).collect(),
            )?;
            if raws.len() != heights.len() {
                return Err(invalid());
            }
            raws.iter()
                .zip(heights)
                .map(|(raw, height)| {
                    let header: BlockHeader = decode(raw)?;
                    if header.height != *height {
                        return Err(invalid());
                    }
                    Ok(header)
                })
                .collect()
        })
    }

    fn get_scripts_history(
        &self,
        scripts: &[&Script],
    ) -> std::result::Result<Vec<Vec<History>>, Error> {
        self.guarded(|rpc| {
            let params = scripts
                .iter()
                .map(|s| script_hash(s).map(|h| json!([h])))
                .collect::<Result<Vec<_>>>()?;
            let histories = rpc.batch("blockchain.scripthash.get_history", params)?;
            if histories.len() != scripts.len() {
                return Err(invalid());
            }
            histories
                .iter()
                .map(|history| {
                    history
                        .as_array()
                        .ok_or_else(invalid)?
                        .iter()
                        .map(|item| {
                            let txid =
                                Txid::from_str(item["tx_hash"].as_str().ok_or_else(invalid)?)
                                    .map_err(|_| invalid())?;
                            let height = crate::scan::integer(&item["height"])
                                .and_then(|n| i32::try_from(n).ok())
                                .ok_or_else(invalid)?;
                            Ok(History {
                                txid,
                                height,
                                block_hash: None,
                                block_timestamp: None,
                                v: 0,
                            })
                        })
                        .collect()
                })
                .collect()
        })
    }
}
