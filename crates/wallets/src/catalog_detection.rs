//! `ui.wallets.detect_script_types`: which script types a bare xpub has used.
//!
//! The probe is the user's explicit "detect" action. It checks receive index
//! 0 of each script type against one backend through `kassiber-net`, under a
//! once-only consent bound to that action, like Python's
//! `detect_active_script_types`. Every backend or probe failure falls back to
//! a manual `p2wpkh` choice with the failure as the reason.
use crate::{descriptors as d, material, scan::script_hash};
use kassiber_chain::{Backend, ElectrumClient, HttpClient};
use kassiber_envelope::{dict, s, AppError};
use kassiber_net::{Consent, Gate};
use kassiber_pyjson::Value as Py;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

type Result<T> = std::result::Result<T, AppError>;

const FALLBACK: &str = "p2wpkh";

/// The local checks Python runs before it resolves a backend.
pub fn validate_material(material: &str) -> Result<()> {
    d::assert_standalone_watch_only(material).map_err(d::Error::into_app)?;
    let prefix = material.chars().take(4).collect::<String>();
    if !["xpub", "tpub"].contains(&prefix.as_str()) {
        return Err(AppError::new(
            "validation",
            "Script-type detection only applies to a bare xpub/tpub",
        )
        .with_hint("A descriptor or ypub/zpub key already carries its script type."));
    }
    material::normalize(material, &json!({"script_types": [FALLBACK]}))
        .map_err(d::Error::into_app)?;
    Ok(())
}

pub fn fallback(reason: &str) -> Py {
    dict([
        ("probed", Py::Bool(false)),
        ("detected", Py::List(vec![])),
        ("active", Py::List(vec![s(FALLBACK)])),
        ("fallback_used", Py::Bool(true)),
        ("reason", s(reason)),
    ])
}

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

/// One detection request: the four index-0 receive scripts and the backend.
pub struct Detection {
    identity: Arc<()>,
    backend: Backend,
    electrum: bool,
    scripts: Vec<(&'static str, String)>,
}

/// Consent for exactly one run of exactly one `Detection`.
pub struct Grant {
    identity: Arc<()>,
    transport: Consent,
    used: AtomicBool,
}

impl Grant {
    pub fn explicit(action: &Detection) -> Self {
        Self {
            identity: action.identity.clone(),
            transport: Consent::explicit_trigger(action.backend.request()),
            used: AtomicBool::new(false),
        }
    }
}

impl Detection {
    /// Local preparation in Python's order: chain/network, the plan, the
    /// backend's chain/network/kind checks, then the derived scripts. Nothing
    /// here contacts the backend.
    pub fn new(
        material: &str,
        chain: &str,
        network: Option<&str>,
        backend: &Value,
    ) -> Result<Self> {
        let chain = d::normalize_chain(chain).map_err(d::Error::into_app)?;
        let mut config = json!({"xpub": material, "script_types": d::SCRIPT_TYPES, "chain": chain, "gap_limit": 1});
        if let Some(network) = network.filter(|n| !n.is_empty()) {
            config["network"] =
                json!(d::normalize_network(&chain, network).map_err(d::Error::into_app)?);
        }
        let plan = d::load(&config)
            .map_err(d::Error::into_app)?
            .ok_or_else(|| AppError::new("app_error", "Script-type detection needs an xpub"))?;
        let name = d::text(backend.get("name").unwrap_or(&Value::Null));
        let kind = backend_kind(&d::text(backend.get("kind").unwrap_or(&Value::Null)));
        let configured = |key| {
            let value = kassiber_pytext::strip(&d::field(backend, key)).to_owned();
            (!value.is_empty()).then_some(value)
        };
        if let Some(value) = configured("chain") {
            let expected = d::normalize_chain(&value).map_err(d::Error::into_app)?;
            if expected != plan.chain {
                return Err(mismatch(&name, &expected, &plan.chain));
            }
        }
        if let Some(value) = configured("network") {
            let expected = d::normalize_network(&plan.chain, &value).map_err(d::Error::into_app)?;
            if expected != plan.network {
                return Err(mismatch(&name, &expected, &plan.network));
            }
        }
        if plan.chain == "liquid" && !["esplora", "electrum"].contains(&kind.as_str()) {
            return Err(AppError::new(
                "app_error",
                "Liquid live refresh currently requires an Esplora-compatible or Electrum backend",
            ));
        }
        if plan.chain != "bitcoin" && kind == "bitcoinrpc" {
            return Err(AppError::new(
                "app_error",
                format!(
                    "Backend kind '{kind}' does not support {} wallets",
                    plan.chain
                ),
            ));
        }
        let mut scripts = vec![];
        for (position, script_type) in d::SCRIPT_TYPES.iter().enumerate() {
            let target = plan
                .derive(Some((position * 2) as u32), 0, 1)
                .map_err(d::Error::into_app)?
                .remove(0);
            scripts.push((
                *script_type,
                target["script_pubkey"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            ));
        }
        if !["esplora", "electrum"].contains(&kind.as_str()) {
            return Err(AppError::new(
                "app_error",
                format!("Script-type detection is not implemented for backend kind '{kind}'"),
            ));
        }
        Ok(Self {
            identity: Arc::new(()),
            backend: Backend::new(backend, kind == "electrum")?,
            electrum: kind == "electrum",
            scripts,
        })
    }

    pub fn run(&self, grant: Option<&Grant>, gate: Gate, deadline: Instant) -> Result<Py> {
        let refused = || {
            AppError::new(
                "interaction_required",
                "Network action requires current consent",
            )
        };
        let grant = grant
            .filter(|g| Arc::ptr_eq(&g.identity, &self.identity))
            .ok_or_else(refused)?;
        if grant.used.swap(true, Ordering::SeqCst) {
            return Err(refused());
        }
        let hashes = self
            .scripts
            .iter()
            .map(|(_, script)| script_hash(script))
            .collect::<Result<Vec<_>>>()?;
        let histories = if self.electrum {
            let mut client = ElectrumClient::connect(
                self.backend.clone(),
                &gate,
                Some(grant.transport.clone()),
                deadline,
            )?;
            client
                .batch_call(
                    &hashes
                        .iter()
                        .map(|hash| ("blockchain.scripthash.subscribe".into(), json!([hash])))
                        .collect::<Vec<_>>(),
                    deadline,
                )?
                .iter()
                .map(|status| !status.is_null())
                .collect::<Vec<_>>()
        } else {
            let client = HttpClient::new(self.backend.clone(), gate, env!("CARGO_PKG_VERSION"));
            let mut used = vec![];
            for hash in &hashes {
                let stats = client.get_json(
                    &format!("scripthash/{hash}"),
                    Some(&grant.transport),
                    deadline,
                )?;
                let count = |key: &str| {
                    let value = &stats[key]["tx_count"];
                    if d::truthy(value) {
                        kassiber_pytext::parse_int(&d::text(value)).map_err(|_| {
                            AppError::new("app_error", "Invalid scripthash statistics")
                        })
                    } else {
                        Ok(kassiber_pyjson::BigInt::default())
                    }
                };
                used.push(
                    count("chain_stats")? + count("mempool_stats")?
                        > kassiber_pyjson::BigInt::default(),
                );
            }
            used
        };
        let mut detected = vec![];
        let mut active = vec![];
        for ((script_type, _), used) in self.scripts.iter().zip(histories) {
            detected.push(dict([
                ("script_type", s(*script_type)),
                ("has_history", Py::Bool(used)),
            ]));
            if used {
                active.push(s(*script_type));
            }
        }
        let fallback_used = active.is_empty();
        if fallback_used {
            active.push(s(FALLBACK));
        }
        Ok(dict([
            ("probed", Py::Bool(true)),
            ("detected", Py::List(detected)),
            ("active", Py::List(active)),
            ("fallback_used", Py::Bool(fallback_used)),
            ("reason", Py::Null),
        ]))
    }
}

fn mismatch(name: &str, expected: &str, required: &str) -> AppError {
    AppError::new(
        "app_error",
        format!(
            "Backend '{name}' is configured for {expected}, but source refresh requires {required}"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::get;
    use kassiber_net::{test_support::FakeProvider, Ledger, Offline};
    use std::time::Duration;

    const XPUB: &str = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";

    fn gate(offline: Offline) -> Gate {
        Gate::new(move || offline, Ledger::default())
    }
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    #[test]
    fn detection_requires_exact_once_only_consent_before_any_request() {
        let server =
            FakeProvider::new(r#"{"chain_stats":{"tx_count":1},"mempool_stats":{"tx_count":0}}"#)
                .unwrap();
        let config = json!({"name": "fake", "kind": "esplora", "url": server.base()});
        let action = Detection::new(XPUB, "bitcoin", None, &config).unwrap();
        let other = Detection::new(XPUB, "bitcoin", None, &config).unwrap();
        let grant = Grant::explicit(&action);
        let refused = |result: Result<Py>| result.unwrap_err().code;
        assert_eq!(
            refused(action.run(None, gate(Offline::Online), deadline())),
            "interaction_required"
        );
        assert_eq!(
            refused(other.run(Some(&grant), gate(Offline::Online), deadline())),
            "interaction_required"
        );
        assert!(server.requests().is_empty());
        let payload = action
            .run(Some(&grant), gate(Offline::Online), deadline())
            .unwrap();
        assert_eq!(server.requests().len(), 4);
        assert_eq!(get(&payload, "fallback_used"), Some(&Py::Bool(false)));
        assert_eq!(
            get(&payload, "active"),
            Some(&Py::List(d::SCRIPT_TYPES.iter().map(|v| s(*v)).collect()))
        );
        assert_eq!(
            refused(action.run(Some(&grant), gate(Offline::Online), deadline())),
            "interaction_required"
        );
        assert_eq!(server.requests().len(), 4);
    }

    #[test]
    fn detection_without_history_falls_back_and_offline_sends_nothing() {
        let server =
            FakeProvider::new(r#"{"chain_stats":{"tx_count":0},"mempool_stats":{"tx_count":0}}"#)
                .unwrap();
        let config = json!({"name": "fake", "kind": "mempool", "url": server.base()});
        let action = Detection::new(XPUB, "bitcoin", Some("main"), &config).unwrap();
        let payload = action
            .run(
                Some(&Grant::explicit(&action)),
                gate(Offline::Online),
                deadline(),
            )
            .unwrap();
        assert_eq!(get(&payload, "fallback_used"), Some(&Py::Bool(true)));
        assert_eq!(get(&payload, "active"), Some(&Py::List(vec![s(FALLBACK)])));
        let requests = server.requests().len();
        let action = Detection::new(XPUB, "bitcoin", None, &config).unwrap();
        assert!(action
            .run(
                Some(&Grant::explicit(&action)),
                gate(Offline::Offline),
                deadline()
            )
            .is_err());
        assert_eq!(server.requests().len(), requests);
    }

    #[test]
    fn detection_checks_backend_scope_and_kind_before_any_transport() {
        let message = |config: Value, network: Option<&str>| {
            Detection::new(XPUB, "bitcoin", network, &config)
                .err()
                .unwrap()
                .message
        };
        assert_eq!(
            message(
                json!({"name": "t", "kind": "esplora", "url": "http://x", "network": "testnet"}),
                None
            ),
            "Backend 't' is configured for test, but source refresh requires main"
        );
        assert_eq!(
            message(
                json!({"name": "l", "kind": "esplora", "url": "http://x", "chain": "liquid"}),
                None
            ),
            "Backend 'l' is configured for liquid, but source refresh requires bitcoin"
        );
        assert_eq!(
            message(
                json!({"name": "c", "kind": "Core", "url": "http://x"}),
                None
            ),
            "Script-type detection is not implemented for backend kind 'bitcoinrpc'"
        );
        assert_eq!(
            message(
                json!({"name": "c", "kind": "electrum", "url": "tcp://x:1"}),
                Some("bogus")
            ),
            "Unsupported bitcoin network 'bogus'"
        );
    }

    #[test]
    fn detection_rejects_private_and_typed_material_without_echoing_it() {
        let secret = bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Bitcoin, &[0u8; 32])
            .unwrap()
            .to_string();
        let error = validate_material(&secret).unwrap_err();
        assert_eq!(error.code, "wallet_spending_private_material");
        assert!(!error.message.contains(&secret));
        assert_eq!(
            validate_material(&format!("wpkh({XPUB})"))
                .unwrap_err()
                .code,
            "validation"
        );
        validate_material(XPUB).unwrap();
    }
}
