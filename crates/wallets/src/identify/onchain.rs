//! The opt-in on-chain tier of identification (`ui.wallets.identify_onchain`).
//!
//! The book work happens in `prepare`; a `Verification` then only looks the
//! pending txids up on one Esplora or Electrum backend and classifies them
//! against the in-memory index, so it can run off the request thread. Every
//! lookup goes through `kassiber_net` under a grant minted for this one
//! request; the desktop's "Verify on chain" action is that consent. It is
//! never an AI tool.
use super::{Legs, LookupError, Plan};
use kassiber_chain::{Backend, ElectrumClient, HttpClient};
use kassiber_envelope::AppError;
use kassiber_net::{Consent, Gate};
use kassiber_pyjson::Value;
use serde_json::Value as Json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, AppError>;

/// Room for one lookup, retries included.
const LOOKUP_DEADLINE: Duration = Duration::from_secs(300);

/// `normalize_backend_kind`.
fn backend_kind(raw: &str) -> String {
    let value = kassiber_pytext::strip(raw).to_lowercase();
    match value.as_str() {
        "bitcoin-core" | "bitcoincore" | "core" => "bitcoinrpc".into(),
        "core-ln" | "core-lightning" => "coreln".into(),
        "liquid-esplora" | "mempool" => "esplora".into(),
        _ => value,
    }
}

fn text(value: &Json) -> String {
    crate::descriptors::text(value)
}

/// The backend a verification uses (`resolve_verify_backend` after the
/// backend lookup itself).
pub struct Verifier {
    backend: Json,
    electrum: bool,
}

impl Verifier {
    pub fn new(backend: &Json) -> Result<Self> {
        let kind = backend_kind(&text(backend.get("kind").unwrap_or(&Json::Null)));
        if !["esplora", "electrum"].contains(&kind.as_str()) {
            return Err(AppError::new(
                "validation",
                format!("On-chain verification needs an Esplora or Electrum backend, not '{kind}'"),
            )
            .with_hint("Use an Esplora or Electrum backend for verification."));
        }
        Ok(Self {
            backend: backend.clone(),
            electrum: kind == "electrum",
        })
    }
}

/// A prepared run: the plan, its backend, and the single-use grant this
/// request carries. Holds no database handle.
pub struct Verification {
    plan: Plan,
    verifier: Verifier,
    gate: Gate,
    context: Value,
    used: AtomicBool,
}

/// What `ui.wallets.identify_onchain` answers at once, or must stream.
pub enum Start {
    Report(Value),
    Verify(Box<Verification>),
}

impl Verification {
    pub fn start(plan: Plan, verifier: Verifier, gate: Gate, context: Value) -> Start {
        if plan.pending.is_empty() {
            let mut report = plan.report();
            kassiber_envelope::set(&mut report, "context", context);
            return Start::Report(report);
        }
        Start::Verify(Box::new(Self {
            plan,
            verifier,
            gate,
            context,
            used: AtomicBool::new(false),
        }))
    }

    pub fn pending(&self) -> usize {
        self.plan.pending.len()
    }

    /// `_run` after its first progress record: open the session, look each
    /// pending txid up, classify, and return the terminal report.
    pub fn run(
        self,
        progress: &mut dyn FnMut(usize, usize) -> Result<()>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value> {
        if self.used.swap(true, Ordering::SeqCst) {
            return Err(AppError::new(
                "interaction_required",
                "Network action requires current consent",
            ));
        }
        // This request is the user's explicit "Verify on chain" action.
        let (mut session, configured_chain) = Session::open(&self.verifier, self.gate.clone())?;
        let mut lookup =
            |txid: &str, chain: &str| session.legs(txid, chain, configured_chain.as_deref());
        let mut report = self.plan.finish(Some(&mut lookup), progress, cancelled)?;
        kassiber_envelope::set(&mut report, "context", self.context);
        kassiber_envelope::set(&mut report, "cancelled", Value::Bool(cancelled()));
        Ok(report)
    }
}

/// `wallets identify --verify-on-chain`: the CLI flag is the user's explicit
/// trigger. As Python's `with verify_session(backend)` around `identify`, the
/// session opens before any local work (an Electrum socket even when nothing
/// ends up pending), then each pending txid is looked up in turn. The report
/// is `finish_identify`'s, with no desktop `context` or `cancelled` field.
pub fn verify_now(
    verifier: Verifier,
    gate: Gate,
    prepare: impl FnOnce() -> Result<Plan>,
) -> Result<Value> {
    let (mut session, configured_chain) = Session::open(&verifier, gate)?;
    let plan = prepare()?;
    let mut lookup =
        |txid: &str, chain: &str| session.legs(txid, chain, configured_chain.as_deref());
    plan.finish(Some(&mut lookup), &mut |_, _| Ok(()), &|| false)
}

enum Session {
    Esplora(Box<(HttpClient, Consent)>),
    Electrum(Box<ElectrumClient>),
}

fn missed(error: AppError) -> LookupError {
    if error.code == "validation" {
        LookupError::Refused(error)
    } else {
        LookupError::Missed(error.message)
    }
}

fn py(value: &Json) -> Value {
    kassiber_custody::codec::from_json(value).unwrap_or(Value::Null)
}

impl Session {
    /// `verify_session(backend)` under a grant for the explicit trigger: one
    /// Electrum connection for the batch, or a stateless Esplora client; and
    /// the backend's configured chain.
    fn open(verifier: &Verifier, gate: Gate) -> Result<(Self, Option<String>)> {
        let backend = Backend::new(&verifier.backend, verifier.electrum)?;
        let consent = Consent::explicit_trigger(backend.request());
        let configured_chain = {
            let value =
                kassiber_pytext::strip(&crate::descriptors::field(&verifier.backend, "chain"))
                    .to_owned();
            (!value.is_empty()).then_some(value)
        };
        let session = if verifier.electrum {
            let deadline = Instant::now() + LOOKUP_DEADLINE;
            Session::Electrum(Box::new(ElectrumClient::connect(
                backend,
                &gate,
                Some(consent),
                deadline,
            )?))
        } else {
            Session::Esplora(Box::new((
                HttpClient::new(backend, gate, env!("CARGO_PKG_VERSION")),
                consent,
            )))
        };
        Ok((session, configured_chain))
    }

    /// `fetch_transaction_legs`.
    fn legs(
        &mut self,
        txid: &str,
        chain: &str,
        configured: Option<&str>,
    ) -> std::result::Result<Option<Legs>, LookupError> {
        let source = if chain.is_empty() {
            configured
        } else {
            Some(chain)
        };
        let chain = match source {
            Some(value) => crate::descriptors::normalize_chain(value)
                .map_err(|e| missed(crate::descriptors::Error::into_app(e)))?,
            None => String::new(),
        };
        let deadline = Instant::now() + LOOKUP_DEADLINE;
        match self {
            Session::Esplora(esplora) => {
                let (client, consent) = esplora.as_ref();
                let tx = client
                    .get_json(&format!("tx/{txid}"), Some(consent), deadline)
                    .map_err(missed)?;
                esplora_legs(&tx, chain)
                    .map(Some)
                    .map_err(LookupError::Missed)
            }
            Session::Electrum(client) => {
                if !["bitcoin", "liquid"].contains(&chain.as_str()) {
                    return Err(LookupError::Refused(
                        AppError::new(
                            "validation",
                            "Electrum on-chain verification needs the backend's chain set to bitcoin or liquid",
                        )
                        .with_hint("Set the backend chain (e.g. --chain liquid) or use an Esplora backend."),
                    ));
                }
                let raw = client
                    .call(
                        "blockchain.transaction.get",
                        Some(&serde_json::json!([txid])),
                        deadline,
                    )
                    .map_err(missed)?;
                verify_transaction_id(&raw, txid, &chain).map_err(missed)?;
                let raw = raw.as_str().ok_or_else(|| {
                    LookupError::Missed(format!(
                        "fromhex() argument must be str, not {}",
                        kassiber_envelope::type_name(&py(&raw))
                    ))
                })?;
                if chain == "liquid" {
                    liquid_legs(raw)
                } else {
                    bitcoin_legs(raw)
                }
                .map(Some)
                .map_err(LookupError::Missed)
            }
        }
    }
}

/// `_legs_from_esplora_tx`.
fn esplora_legs(tx: &Json, chain: String) -> std::result::Result<Legs, String> {
    let mut inputs = vec![];
    for vin in tx.get("vin").and_then(Json::as_array).into_iter().flatten() {
        let Some(vin) = vin.as_object() else { continue };
        let prevout = match vin.get("prevout") {
            Some(value) if crate::descriptors::truthy(value) => value,
            _ => &Json::Null,
        };
        if !prevout.is_null() && !prevout.is_object() {
            return Err(format!(
                "'{}' object has no attribute 'get'",
                kassiber_envelope::type_name(&py(prevout))
            ));
        }
        let outpoint = match (vin.get("txid"), vin.get("vout")) {
            (Some(id), Some(vout)) if !id.is_null() && !vout.is_null() => {
                let vout = match vout {
                    Json::Number(n) if n.is_u64() || n.is_i64() => n.to_string(),
                    Json::Bool(flag) => u8::from(*flag).to_string(),
                    other => kassiber_pytext::parse_int(&text(other))
                        .map(|n| n.to_string())
                        .map_err(|_| {
                            format!(
                                "invalid literal for int() with base 10: {}",
                                kassiber_envelope::py_repr(&py(other))
                            )
                        })?,
                };
                Value::from(format!("{}:{vout}", text(id).to_lowercase()))
            }
            _ => Value::Null,
        };
        inputs.push((
            outpoint,
            py(prevout.get("scriptpubkey").unwrap_or(&Json::Null)),
        ));
    }
    let mut outputs = vec![];
    for (position, vout) in tx
        .get("vout")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(vout) = vout.as_object() {
            outputs.push((
                Value::from(position as i64),
                py(vout.get("scriptpubkey").unwrap_or(&Json::Null)),
            ));
        }
    }
    Ok(Legs {
        inputs,
        outputs,
        chain,
        network: String::new(),
        source: "chain".into(),
    })
}

/// `verify_electrum_transaction_id`: fetched bytes must be a complete,
/// canonical encoding of the requested transaction (witness-free txid on
/// Bitcoin, the native identity on Liquid) before they are used.
fn verify_transaction_id(raw: &Json, txid: &str, chain: &str) -> std::result::Result<(), AppError> {
    let invalid = || {
        AppError::new(
            "invalid_observation",
            "Electrum returned invalid transaction bytes",
        )
    };
    let payload = raw
        .as_str()
        .and_then(|raw| hex_bytes(raw).ok())
        .ok_or_else(invalid)?;
    let actual = if chain == "liquid" {
        use elements_miniscript::elements;
        let tx: elements::Transaction =
            elements::encode::deserialize(&payload).map_err(|_| invalid())?;
        if elements::encode::serialize(&tx) != payload {
            return Err(invalid());
        }
        tx.txid().to_string()
    } else {
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&payload).map_err(|_| invalid())?;
        if bitcoin::consensus::serialize(&tx) != payload {
            return Err(invalid());
        }
        tx.compute_txid().to_string()
    };
    if actual.to_lowercase() != txid.to_lowercase() {
        return Err(AppError::new(
            "invalid_observation",
            "Electrum returned different transaction bytes",
        ));
    }
    Ok(())
}

fn hex_bytes(raw: &str) -> std::result::Result<Vec<u8>, String> {
    let bytes: Vec<u8> = raw
        .bytes()
        .filter(|b| !b" \t\n\r\x0b\x0c".contains(b))
        .collect();
    if !super::codec::is_hex(raw) {
        return Err("non-hexadecimal number found in fromhex() arg".into());
    }
    Ok((0..bytes.len() / 2)
        .map(|i| {
            u8::from_str_radix(std::str::from_utf8(&bytes[2 * i..2 * i + 2]).unwrap(), 16).unwrap()
        })
        .collect())
}

fn varint(payload: &[u8], offset: &mut usize) -> std::result::Result<u64, String> {
    let short = || "index out of range".to_owned();
    let first = *payload.get(*offset).ok_or_else(short)?;
    *offset += 1;
    let width = match first {
        0xfd => 2,
        0xfe => 4,
        0xff => 8,
        value => return Ok(u64::from(value)),
    };
    let bytes = payload.get(*offset..*offset + width).ok_or_else(short)?;
    *offset += width;
    Ok(bytes
        .iter()
        .rev()
        .fold(0, |acc, b| acc << 8 | u64::from(*b)))
}

/// `decode_raw_transaction` reduced to `_legs_from_bitcoin_tx`. Like Python,
/// short fields read what is there instead of failing.
fn bitcoin_legs(raw: &str) -> std::result::Result<Legs, String> {
    let payload = hex_bytes(raw)?;
    let little = |bytes: &[u8]| {
        bytes
            .iter()
            .rev()
            .fold(0u64, |acc, b| acc << 8 | u64::from(*b))
    };
    let slice = |start: usize, len: usize| {
        &payload[start.min(payload.len())..(start + len).min(payload.len())]
    };
    let mut offset = 4;
    if payload.len() > offset + 1 && payload[offset] == 0 && payload[offset + 1] != 0 {
        offset += 2;
    }
    let mut inputs = vec![];
    for _ in 0..varint(&payload, &mut offset)? {
        let txid: String = slice(offset, 32)
            .iter()
            .rev()
            .map(|b| format!("{b:02x}"))
            .collect();
        let vout = little(slice(offset + 32, 4));
        offset += 36;
        offset += usize::try_from(varint(&payload, &mut offset)?)
            .map_err(|_| "index out of range".to_owned())?;
        offset += 4;
        inputs.push((Value::from(format!("{txid}:{vout}")), Value::Null));
    }
    let mut outputs = vec![];
    for index in 0..varint(&payload, &mut offset)? {
        offset += 8;
        let length = usize::try_from(varint(&payload, &mut offset)?)
            .map_err(|_| "index out of range".to_owned())?;
        let script: String = slice(offset, length)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        offset += length;
        outputs.push((Value::from(index as i64), Value::from(script)));
    }
    Ok(Legs {
        inputs,
        outputs,
        chain: "bitcoin".into(),
        network: String::new(),
        source: "chain".into(),
    })
}

/// `decode_liquid_transaction` reduced to `_legs_from_liquid_tx`.
fn liquid_legs(raw: &str) -> std::result::Result<Legs, String> {
    use elements_miniscript::elements;
    let payload = hex_bytes(raw)?;
    let tx: elements::Transaction = elements::encode::deserialize(&payload)
        .map_err(|_| "Invalid Liquid transaction".to_owned())?;
    let inputs = tx
        .input
        .iter()
        .map(|vin| {
            (
                Value::from(format!(
                    "{}:{}",
                    vin.previous_output.txid, vin.previous_output.vout
                )),
                Value::Null,
            )
        })
        .collect();
    let outputs = tx
        .output
        .iter()
        .enumerate()
        .map(|(index, out)| {
            let script: String = out
                .script_pubkey
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            (
                Value::from(index as i64),
                if script.is_empty() {
                    Value::Null
                } else {
                    Value::from(script)
                },
            )
        })
        .collect();
    Ok(Legs {
        inputs,
        outputs,
        chain: "liquid".into(),
        network: String::new(),
        source: "chain".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_bitcoin_transactions_reduce_to_outpoints_and_scripts() {
        // One input spending 11…11:1, outputs to a P2WPKH script and OP_RETURN.
        let raw = format!(
            "02000000000101{}0100000000ffffffff02e803000000000000160014{}0000000000000000026a00024730440220{}0220{}012102{}00000000",
            "11".repeat(32),
            "ab".repeat(20),
            "01".repeat(32),
            "02".repeat(32),
            "03".repeat(32),
        );
        let legs = bitcoin_legs(&raw).unwrap();
        assert_eq!(
            legs.inputs,
            [(Value::from(format!("{}:1", "11".repeat(32))), Value::Null)]
        );
        assert_eq!(
            legs.outputs,
            [
                (
                    Value::from(0),
                    Value::from(format!("0014{}", "ab".repeat(20)))
                ),
                (Value::from(1), Value::from("6a00")),
            ]
        );
        assert!(bitcoin_legs("zz").is_err());
    }

    #[test]
    fn esplora_transactions_keep_prevout_scripts() {
        let tx = serde_json::json!({
            "vin": [{"txid": "AA", "vout": 2, "prevout": {"scriptpubkey": "0014"}}, {"txid": "bb", "vout": null}, 3],
            "vout": [5, {"scriptpubkey": "51"}],
        });
        let legs = esplora_legs(&tx, String::new()).unwrap();
        assert_eq!(
            legs.inputs,
            [
                (Value::from("aa:2"), Value::from("0014")),
                (Value::Null, Value::Null)
            ]
        );
        assert_eq!(legs.outputs, [(Value::from(1), Value::from("51"))]);
        assert_eq!(legs.source, "chain");
    }

    #[test]
    fn verification_needs_an_esplora_or_electrum_backend() {
        let error = Verifier::new(&serde_json::json!({"kind": "Core"}))
            .err()
            .unwrap();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            (
                "validation",
                "On-chain verification needs an Esplora or Electrum backend, not 'bitcoinrpc'"
            )
        );
        assert!(Verifier::new(&serde_json::json!({"kind": "liquid-esplora"})).is_ok());
    }
}
