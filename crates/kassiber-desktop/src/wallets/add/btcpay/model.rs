//! btcpaySetupModel.ts: the pure model of the BTCPay Server connection setup.
//!
//! The core inspects the merchant's server and returns a plan (`ui.connections.btcpay.discover`)
//! with a recommendation per store payment method. This module only turns that plan plus the
//! user's choices into the `ui.connections.btcpay.create` payload; the core re-validates every
//! route when the setup is created.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const ACTIONS: [&str; 5] = [
    "wallet_source",
    "existing_wallet",
    "payment_ledger",
    "provenance_only",
    "skip",
];
pub const DEFAULT_PAYMENT_METHOD_ID: &str = "BTC-CHAIN";

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Availability {
    pub available: bool,
    pub reason: Option<String>,
    pub reason_text: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Recommendation {
    pub action: String,
    pub wallet: Option<String>,
    pub reason: Option<String>,
    pub reason_text: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExistingRoute {
    pub action: String,
    pub backend: Option<String>,
    pub wallet: Option<String>,
    pub wallet_id: Option<String>,
    pub store_id: String,
    pub payment_method_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct OwnedBy {
    pub wallet: String,
    pub wallet_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct StoreRef {
    pub store_id: String,
    pub payment_method_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct LightningWallet {
    pub wallet: String,
    pub kind: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Method {
    pub store_id: String,
    pub payment_method_id: String,
    pub label: String,
    pub enabled: bool,
    pub sync_supported: bool,
    pub rail: String,
    pub currency: Option<String>,
    pub chain: Option<String>,
    pub bitcoin_asset: Option<bool>,
    pub settlement: Option<String>,
    pub settlement_hint: Option<String>,
    pub ledger_group: Option<String>,
    pub wallet_history_available: Option<bool>,
    pub wallet_fingerprint: Option<String>,
    pub network: Option<String>,
    pub preview_error: Option<String>,
    pub network_compatible: Option<bool>,
    pub existing_routes: Vec<ExistingRoute>,
    pub configured_via: Vec<String>,
    pub owned_by: Option<OwnedBy>,
    pub shared_with: Vec<StoreRef>,
    pub lightning_wallets: Vec<LightningWallet>,
    pub actions: Option<HashMap<String, Availability>>,
    pub recommendation: Option<Recommendation>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct StoreSyncState {
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error_code: Option<String>,
    pub age_seconds: Option<f64>,
    pub stale: bool,
    pub never_synced: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Store {
    pub id: String,
    pub name: String,
    pub default_currency: Option<String>,
    pub archived: Option<bool>,
    /// capability → granted (absent / null = unknown).
    pub capabilities: Option<HashMap<String, Option<bool>>>,
    pub missing_permissions: Vec<String>,
    pub payment_methods_error: Option<String>,
    pub sync_state: Option<StoreSyncState>,
}

impl Store {
    pub fn capability(&self, c: &str) -> Option<bool> {
        self.capabilities
            .as_ref()
            .and_then(|m| m.get(c).copied().flatten())
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Warning {
    pub code: String,
    pub message: String,
    pub severity: Option<String>,
    pub store_id: Option<String>,
    pub store_ids: Option<Vec<String>>,
    pub wallet_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Server {
    pub version: Option<String>,
    pub fully_synced: Option<bool>,
    pub tor: bool,
    pub loopback: bool,
    pub transport: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ApiKey {
    pub label: Option<String>,
    pub known: bool,
    pub scope: String,
    pub store_ids: Option<Vec<String>>,
    pub permissions: Option<Vec<String>>,
    pub risk: String,
    pub excess_permissions: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct KeyGuide {
    pub preset: String,
    pub authorize_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Plan {
    pub backend: String,
    pub server: Option<Server>,
    pub api_key: Option<ApiKey>,
    pub stores: Vec<Store>,
    pub payment_methods: Vec<Method>,
    pub warnings: Vec<Warning>,
    pub sibling_backends: Vec<String>,
    pub existing_routes: Vec<ExistingRoute>,
    pub detected_network: Option<String>,
    pub key_upgrade: Option<KeyGuide>,
    pub key_read_only: Option<KeyGuide>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    pub action: String,
    pub wallet: String,
}

impl Choice {
    pub fn new(action: &str, wallet: &str) -> Choice {
        Choice {
            action: action.into(),
            wallet: wallet.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WalletOption {
    pub label: String,
    pub kind: Option<String>,
    pub chain: Option<String>,
    pub sync_source: Option<String>,
}

impl WalletOption {
    /// One `ui.wallets.list` wallet.
    pub fn from_value(v: &Value) -> WalletOption {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        WalletOption {
            label: s("label").unwrap_or_default(),
            kind: s("kind"),
            chain: s("chain").filter(|c| !c.is_empty()),
            sync_source: s("sync_source"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManualRoute {
    pub store_id: String,
    pub payment_method_id: String,
    pub action: String,
    pub wallet: String,
}

/// `BtcpayInstanceArgs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Instance {
    Saved {
        backend: String,
    },
    New {
        backend_label: String,
        server_url: String,
        api_key: String,
    },
}

impl Instance {
    pub fn args(&self) -> Map<String, Value> {
        let mut m = Map::new();
        match self {
            Instance::Saved { backend } => {
                m.insert("backend".into(), json!(backend));
            }
            Instance::New {
                backend_label,
                server_url,
                api_key,
            } => {
                m.insert("backend_label".into(), json!(backend_label));
                m.insert("server_url".into(), json!(server_url));
                m.insert("api_key".into(), json!(api_key));
            }
        }
        m
    }
}

pub fn route_key(store_id: &str, payment_method_id: &str) -> String {
    format!("{store_id}\u{0}{payment_method_id}")
}

/// Wallets BTCPay can be mapped onto: the wallets that hold the funds.
pub fn settlement_wallet_options(wallets: &[WalletOption]) -> Vec<WalletOption> {
    wallets
        .iter()
        .filter(|w| w.sync_source.as_deref() != Some("btcpay"))
        .cloned()
        .collect()
}

/// Settlement wallets on the same chain as the payment method.
pub fn wallet_options_for_method(
    chain: Option<&str>,
    payment_method_id: &str,
    wallets: &[WalletOption],
) -> Vec<WalletOption> {
    let options = settlement_wallet_options(wallets);
    let chain = chain.map(str::to_owned).unwrap_or_else(|| {
        if payment_method_id.to_uppercase().starts_with("LBTC") {
            "liquid".into()
        } else {
            "bitcoin".into()
        }
    });
    let same: Vec<WalletOption> = options
        .iter()
        .filter(|w| w.chain.as_deref().is_none_or(|c| c == chain))
        .cloned()
        .collect();
    if same.is_empty() { options } else { same }
}

pub fn action_available(m: &Method, action: &str) -> bool {
    if let Some(entry) = m.actions.as_ref().and_then(|a| a.get(action)) {
        return entry.available;
    }
    match action {
        "wallet_source" | "existing_wallet" => m.sync_supported && m.rail == "onchain",
        "payment_ledger" => m.ledger_group.as_deref().is_some_and(|g| !g.is_empty()),
        _ => true,
    }
}

pub fn default_route_choice(m: &Method, wallets: &[WalletOption]) -> Choice {
    let recommended = m
        .recommendation
        .as_ref()
        .map(|r| r.action.as_str())
        .filter(|a| !a.is_empty());
    let action = match recommended {
        Some(a) if action_available(m, a) => a,
        _ if action_available(m, "provenance_only") => "provenance_only",
        _ => "skip",
    };
    let candidates = wallet_options_for_method(m.chain.as_deref(), &m.payment_method_id, wallets);
    let preferred = m
        .recommendation
        .as_ref()
        .and_then(|r| r.wallet.clone())
        .or_else(|| m.owned_by.as_ref().map(|o| o.wallet.clone()));
    let wallet = match preferred {
        Some(p) if candidates.iter().any(|c| c.label == p) => p,
        _ if candidates.len() == 1 => candidates[0].label.clone(),
        _ => String::new(),
    };
    Choice {
        action: action.into(),
        wallet,
    }
}

pub fn default_route_choices(plan: &Plan, wallets: &[WalletOption]) -> HashMap<String, Choice> {
    plan.payment_methods
        .iter()
        .map(|m| {
            (
                route_key(&m.store_id, &m.payment_method_id),
                default_route_choice(m, wallets),
            )
        })
        .collect()
}

pub fn choice_for_method(
    m: &Method,
    choices: &HashMap<String, Choice>,
    wallets: &[WalletOption],
) -> Choice {
    match choices.get(&route_key(&m.store_id, &m.payment_method_id)) {
        Some(c) if action_available(m, &c.action) => c.clone(),
        _ => default_route_choice(m, wallets),
    }
}

#[derive(Clone, Debug)]
pub struct Entry<'a> {
    pub key: String,
    pub method: &'a Method,
    pub store: Option<&'a Store>,
    pub choice: Choice,
}

pub fn plan_routes<'a>(
    plan: &'a Plan,
    choices: &HashMap<String, Choice>,
    wallets: &[WalletOption],
) -> Vec<Entry<'a>> {
    plan.payment_methods
        .iter()
        .map(|m| Entry {
            key: route_key(&m.store_id, &m.payment_method_id),
            method: m,
            store: plan.stores.iter().find(|s| s.id == m.store_id),
            choice: choice_for_method(m, choices, wallets),
        })
        .collect()
}

/// Counts per action, in `ACTIONS` order.
pub fn summarize_routes(entries: &[Entry]) -> [usize; 5] {
    let mut counts = [0usize; 5];
    for e in entries {
        if let Some(i) = ACTIONS.iter().position(|a| *a == e.choice.action) {
            counts[i] += 1;
        }
    }
    counts
}

pub fn validate(
    plan: Option<&Plan>,
    entries: &[Entry],
    manual: Option<&ManualRoute>,
    wallets: &[WalletOption],
) -> Option<&'static str> {
    let Some(plan) = plan else {
        return Some("discover_first");
    };
    if plan.payment_methods.is_empty() {
        let Some(m) = manual else {
            return Some("no_routes");
        };
        if m.store_id.trim().is_empty() {
            return Some("manual_store_id");
        }
        if m.action == "skip" {
            return Some("no_routes");
        }
        if m.action == "existing_wallet" {
            if settlement_wallet_options(wallets).is_empty() {
                return Some("no_wallets");
            }
            if m.wallet.is_empty() {
                return Some("choose_wallet");
            }
        }
        return None;
    }
    let active: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.choice.action != "skip")
        .collect();
    let changes_existing = entries
        .iter()
        .any(|e| e.choice.action == "skip" && !e.method.existing_routes.is_empty());
    if active.is_empty() && !changes_existing {
        return Some("no_routes");
    }
    let mapped: Vec<&&Entry> = active
        .iter()
        .filter(|e| e.choice.action == "existing_wallet")
        .collect();
    if !mapped.is_empty() {
        if settlement_wallet_options(wallets).is_empty() {
            return Some("no_wallets");
        }
        if mapped.iter().any(|e| e.choice.wallet.is_empty()) {
            return Some("choose_wallet");
        }
    }
    None
}

/// `buildCreateRoutes`: each route as the JSON object the core takes.
pub fn build_create_routes(
    plan: &Plan,
    entries: &[Entry],
    manual: Option<&ManualRoute>,
    connection_label: &str,
) -> Vec<Value> {
    if plan.payment_methods.is_empty() {
        let Some(m) = manual else { return vec![] };
        let pmid = match m.payment_method_id.trim().to_uppercase() {
            s if s.is_empty() => DEFAULT_PAYMENT_METHOD_ID.to_owned(),
            s => s,
        };
        let mut r = Map::new();
        r.insert("store_id".into(), json!(m.store_id.trim()));
        r.insert("payment_method_id".into(), json!(pmid));
        r.insert("action".into(), json!(m.action));
        if m.action == "existing_wallet" {
            r.insert("wallet".into(), json!(m.wallet));
        }
        if m.action == "wallet_source" {
            r.insert("label".into(), json!(connection_label.trim()));
        }
        if let Some(n) = &plan.detected_network {
            r.insert("network".into(), json!(n));
        }
        return vec![Value::Object(r)];
    }
    let wallet_sources = entries
        .iter()
        .filter(|e| e.choice.action == "wallet_source")
        .count();
    entries
        .iter()
        // Re-sending an unchanged route is idempotent in the core; a skip is only sent when it
        // removes a route that exists today.
        .filter(|e| e.choice.action != "skip" || !e.method.existing_routes.is_empty())
        .map(|e| {
            let mut r = Map::new();
            r.insert("store_id".into(), json!(e.method.store_id));
            if let Some(name) = e.store.map(|s| s.name.as_str()).filter(|n| !n.is_empty()) {
                r.insert("store_name".into(), json!(name));
            }
            r.insert(
                "payment_method_id".into(),
                json!(e.method.payment_method_id),
            );
            r.insert("action".into(), json!(e.choice.action));
            if e.choice.action == "existing_wallet" {
                r.insert("wallet".into(), json!(e.choice.wallet));
            }
            if e.choice.action == "skip"
                && let Some(w) = e
                    .method
                    .existing_routes
                    .iter()
                    .find(|x| {
                        x.action == "existing_wallet"
                            && x.wallet.as_deref().is_some_and(|w| !w.is_empty())
                    })
                    .and_then(|x| x.wallet.clone())
            {
                r.insert("wallet".into(), json!(w));
            }
            if e.choice.action == "wallet_source" && wallet_sources == 1 {
                r.insert("label".into(), json!(connection_label.trim()));
            }
            if let Some(fp) = e
                .method
                .wallet_fingerprint
                .as_deref()
                .filter(|f| !f.is_empty())
            {
                r.insert("wallet_fingerprint".into(), json!(fp));
            }
            // Lightning and plugin rails expose no addresses; a ledger takes the server's
            // on-chain network.
            let network = e.method.network.clone().or_else(|| {
                if e.choice.action == "payment_ledger" {
                    plan.detected_network.clone()
                } else {
                    None
                }
            });
            if let Some(n) = network.filter(|n| !n.is_empty()) {
                r.insert("network".into(), json!(n));
            }
            Value::Object(r)
        })
        .collect()
}

pub fn build_create_args(
    instance: &Instance,
    is_new_instance: bool,
    plan: &Plan,
    routes: &[Value],
    connection_label: &str,
) -> Value {
    let mut m = instance.args();
    m.insert("mode".into(), json!("account"));
    m.insert("label".into(), json!(connection_label.trim()));
    m.insert("routes".into(), Value::Array(routes.to_vec()));
    m.insert("sync_provenance".into(), json!(true));
    if is_new_instance && let Some(n) = &plan.detected_network {
        m.insert("network".into(), json!(n));
    }
    Value::Object(m)
}

/// Wallets the post-create refresh should sync: (wallet sources, mapped wallets).
pub fn wallets_to_refresh(
    routes: &[Value],
    created_wallet_sources: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut mapped: Vec<String> = Vec::new();
    for r in routes {
        if r["action"] == "existing_wallet"
            && let Some(w) = r["wallet"].as_str().filter(|w| !w.is_empty())
            && !mapped.iter().any(|x| x == w)
        {
            mapped.push(w.to_owned());
        }
    }
    (created_wallet_sources.to_vec(), mapped)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warning,
    Danger,
    Neutral,
}

pub fn key_risk_tone(risk: Option<&str>) -> Tone {
    match risk {
        Some("read_only") => Tone::Good,
        Some("can_write" | "can_modify_store") => Tone::Warning,
        Some("server_admin") => Tone::Danger,
        _ => Tone::Neutral,
    }
}

/// `WARNING_TONES`.
pub fn warning_tone(code: &str) -> Tone {
    match code {
        "key_server_admin" | "unencrypted_transport" => Tone::Danger,
        "key_can_modify_store"
        | "key_can_write"
        | "server_not_synced"
        | "store_catalog_unavailable"
        | "no_store_access"
        | "stale_store_data" => Tone::Warning,
        _ => Tone::Neutral,
    }
}

pub fn rail_kind(rail: &str, chain: Option<&str>, bitcoin_asset: Option<bool>) -> &'static str {
    if bitcoin_asset == Some(false) {
        return if rail == "plugin" {
            "plugin"
        } else {
            "other_asset"
        };
    }
    match rail {
        "onchain" => {
            if chain == Some("liquid") {
                "liquid"
            } else {
                "onchain"
            }
        }
        "lightning" => "lightning",
        "lnurl" => "lnurl",
        _ => "plugin",
    }
}

/// Basic client-side check before asking the core for a key link (`new URL` with a hostname).
pub fn looks_like_server_url(value: &str) -> bool {
    let t = value.trim();
    let lower = t.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        host_port
            .split(']')
            .next()
            .map(|h| h.trim_start_matches('['))
            .unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    !host.is_empty() && !host.contains(char::is_whitespace)
}

pub fn store_name(plan: Option<&Plan>, store_id: &str) -> String {
    plan.and_then(|p| p.stores.iter().find(|s| s.id == store_id))
        .map(|s| s.name.clone())
        .unwrap_or_else(|| store_id.to_owned())
}

pub fn store_freshness(state: Option<&StoreSyncState>) -> Option<&'static str> {
    let s = state?;
    Some(if s.never_synced {
        "never"
    } else if s.last_error_code.is_some() {
        "failed"
    } else if s.stale {
        "stale"
    } else {
        "fresh"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const POPUP: &str = "store-popup";
    const CAFE: &str = "store-cafe";
    const MAIN: &str = "store-main";

    fn avail(entries: &[(&str, bool, Option<&str>)]) -> Option<HashMap<String, Availability>> {
        Some(
            entries
                .iter()
                .map(|(a, ok, r)| {
                    (
                        a.to_string(),
                        Availability {
                            available: *ok,
                            reason: r.map(str::to_owned),
                            reason_text: None,
                        },
                    )
                })
                .collect(),
        )
    }

    fn method() -> Method {
        Method {
            store_id: MAIN.into(),
            payment_method_id: "BTC-CHAIN".into(),
            label: "Bitcoin on-chain".into(),
            enabled: true,
            sync_supported: true,
            rail: "onchain".into(),
            chain: Some("bitcoin".into()),
            bitcoin_asset: Some(true),
            settlement: Some("btc_onchain".into()),
            network: Some("regtest".into()),
            actions: avail(&[
                ("wallet_source", true, None),
                ("existing_wallet", true, None),
                ("provenance_only", true, None),
                ("skip", true, None),
            ]),
            recommendation: Some(Recommendation {
                action: "wallet_source".into(),
                wallet: None,
                reason: Some("import_btcpay_wallet_history".into()),
                reason_text: None,
            }),
            ..Method::default()
        }
    }

    fn rec(action: &str, wallet: Option<&str>, reason: &str) -> Option<Recommendation> {
        Some(Recommendation {
            action: action.into(),
            wallet: wallet.map(str::to_owned),
            reason: Some(reason.into()),
            reason_text: None,
        })
    }

    fn read_only_plan() -> Plan {
        let owned = Some(OwnedBy {
            wallet: "Shop Trezor".into(),
            wallet_id: "w-1".into(),
        });
        Plan {
            backend: "shop-btcpay".into(),
            api_key: Some(ApiKey {
                known: true,
                scope: "all_stores".into(),
                risk: "read_only".into(),
                label: Some("kassiber-ro-all".into()),
                ..ApiKey::default()
            }),
            detected_network: Some("regtest".into()),
            stores: vec![
                Store {
                    id: POPUP.into(),
                    name: "Pop-up Stand".into(),
                    ..Store::default()
                },
                Store {
                    id: CAFE.into(),
                    name: "Café".into(),
                    ..Store::default()
                },
                Store {
                    id: MAIN.into(),
                    name: "Main Store".into(),
                    ..Store::default()
                },
            ],
            payment_methods: vec![
                Method {
                    store_id: POPUP.into(),
                    wallet_fingerprint: Some("btcpay-wallet:shared".into()),
                    owned_by: owned.clone(),
                    shared_with: vec![StoreRef {
                        store_id: MAIN.into(),
                        payment_method_id: "BTC-CHAIN".into(),
                    }],
                    actions: avail(&[
                        (
                            "wallet_source",
                            false,
                            Some("wallet_history_permission_missing"),
                        ),
                        ("existing_wallet", true, None),
                        ("provenance_only", true, None),
                        ("skip", true, None),
                    ]),
                    recommendation: rec(
                        "existing_wallet",
                        Some("Shop Trezor"),
                        "wallet_recognised",
                    ),
                    ..method()
                },
                Method {
                    store_id: CAFE.into(),
                    payment_method_id: "BTC-LN".into(),
                    label: "Lightning".into(),
                    sync_supported: false,
                    rail: "lightning".into(),
                    network: None,
                    settlement: Some("lightning".into()),
                    ledger_group: Some("lightning".into()),
                    actions: avail(&[
                        ("wallet_source", false, Some("not_on_chain")),
                        ("existing_wallet", false, Some("not_on_chain")),
                        ("payment_ledger", true, None),
                        ("provenance_only", true, None),
                        ("skip", true, None),
                    ]),
                    recommendation: rec("payment_ledger", None, "lightning_payment_ledger"),
                    ..method()
                },
                Method {
                    store_id: CAFE.into(),
                    wallet_fingerprint: Some("btcpay-wallet:cafe".into()),
                    actions: avail(&[
                        (
                            "wallet_source",
                            false,
                            Some("wallet_history_permission_missing"),
                        ),
                        ("existing_wallet", true, None),
                        ("provenance_only", true, None),
                        ("skip", true, None),
                    ]),
                    recommendation: rec("provenance_only", None, "read_only_key_upgrade"),
                    ..method()
                },
                Method {
                    store_id: MAIN.into(),
                    wallet_fingerprint: Some("btcpay-wallet:shared".into()),
                    owned_by: owned,
                    shared_with: vec![StoreRef {
                        store_id: POPUP.into(),
                        payment_method_id: "BTC-CHAIN".into(),
                    }],
                    recommendation: rec(
                        "existing_wallet",
                        Some("Shop Trezor"),
                        "wallet_recognised",
                    ),
                    ..method()
                },
            ],
            ..Plan::default()
        }
    }

    fn wallets() -> Vec<WalletOption> {
        let w = |l: &str, k: &str, c: &str, s: &str| WalletOption {
            label: l.into(),
            kind: Some(k.into()),
            chain: Some(c.into()),
            sync_source: Some(s.into()),
        };
        vec![
            w("Shop Trezor", "descriptor", "bitcoin", ""),
            w("Liquid Hot", "descriptor", "liquid", ""),
            w("Old BTCPay Source", "custom", "bitcoin", "btcpay"),
        ]
    }

    #[test]
    fn preselects_recommendation_and_recognised_wallet() {
        let plan = read_only_plan();
        let c = default_route_choices(&plan, &wallets());
        assert_eq!(
            c[&route_key(POPUP, "BTC-CHAIN")],
            Choice::new("existing_wallet", "Shop Trezor")
        );
        assert_eq!(c[&route_key(CAFE, "BTC-LN")].action, "payment_ledger");
        assert_eq!(c[&route_key(CAFE, "BTC-CHAIN")].action, "provenance_only");
    }

    #[test]
    fn never_keeps_unavailable_action() {
        let plan = read_only_plan();
        let mut ch = HashMap::new();
        ch.insert(
            route_key(POPUP, "BTC-CHAIN"),
            Choice::new("wallet_source", ""),
        );
        let e = plan_routes(&plan, &ch, &wallets());
        assert_eq!(e[0].choice.action, "existing_wallet");
    }

    #[test]
    fn builds_routes_with_fingerprints_networks_store_names() {
        let plan = read_only_plan();
        let w = wallets();
        let e = plan_routes(&plan, &default_route_choices(&plan, &w), &w);
        let r = build_create_routes(&plan, &e, None, "Shop");
        assert_eq!(r.len(), 4);
        assert_eq!(
            r[0],
            json!({"store_id": POPUP, "store_name": "Pop-up Stand", "payment_method_id": "BTC-CHAIN", "action": "existing_wallet", "wallet": "Shop Trezor", "wallet_fingerprint": "btcpay-wallet:shared", "network": "regtest"})
        );
        assert_eq!(
            r[1],
            json!({"store_id": CAFE, "store_name": "Café", "payment_method_id": "BTC-LN", "action": "payment_ledger", "network": "regtest"})
        );
        assert!(r[2].get("wallet").is_none());
    }

    #[test]
    fn drops_skipped_unless_removing_existing() {
        let mut plan = read_only_plan();
        plan.payment_methods = vec![
            Method {
                store_id: MAIN.into(),
                ..method()
            },
            Method {
                store_id: CAFE.into(),
                existing_routes: vec![ExistingRoute {
                    action: "existing_wallet".into(),
                    wallet: Some("Shop Trezor".into()),
                    store_id: CAFE.into(),
                    payment_method_id: "BTC-CHAIN".into(),
                    ..ExistingRoute::default()
                }],
                ..method()
            },
        ];
        let mut skip = HashMap::new();
        skip.insert(route_key(MAIN, "BTC-CHAIN"), Choice::new("skip", ""));
        skip.insert(route_key(CAFE, "BTC-CHAIN"), Choice::new("skip", ""));
        let w = wallets();
        let e = plan_routes(&plan, &skip, &w);
        let r = build_create_routes(&plan, &e, None, "Shop");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0]["store_id"], CAFE);
        assert_eq!(r[0]["action"], "skip");
        assert_eq!(r[0]["wallet"], "Shop Trezor");
        assert_eq!(validate(Some(&plan), &e, None, &w), None);
    }

    #[test]
    fn names_single_imported_wallet_after_connection() {
        let mut plan = read_only_plan();
        plan.payment_methods = vec![Method {
            store_id: MAIN.into(),
            ..method()
        }];
        let w = wallets();
        let e = plan_routes(&plan, &default_route_choices(&plan, &w), &w);
        let r = build_create_routes(&plan, &e, None, " Shop ");
        assert_eq!(r[0]["action"], "wallet_source");
        assert_eq!(r[0]["label"], "Shop");
    }

    #[test]
    fn validates_missing_plans_wallets_and_empty_selections() {
        let w = wallets();
        assert_eq!(validate(None, &[], None, &w), Some("discover_first"));
        let plan = read_only_plan();
        let e = plan_routes(&plan, &default_route_choices(&plan, &w), &w);
        assert_eq!(validate(Some(&plan), &e, None, &w), None);
        assert_eq!(validate(Some(&plan), &e, None, &[]), Some("no_wallets"));
        let unchosen: Vec<Entry> = e
            .iter()
            .cloned()
            .map(|mut x| {
                if x.choice.action == "existing_wallet" {
                    x.choice.wallet.clear();
                }
                x
            })
            .collect();
        assert_eq!(
            validate(Some(&plan), &unchosen, None, &w),
            Some("choose_wallet")
        );
        let skipped: Vec<Entry> = e
            .iter()
            .cloned()
            .map(|mut x| {
                x.choice = Choice::new("skip", "");
                x
            })
            .collect();
        assert_eq!(validate(Some(&plan), &skipped, None, &w), Some("no_routes"));
    }

    #[test]
    fn manual_store_route() {
        let mut plan = read_only_plan();
        plan.payment_methods.clear();
        let w = wallets();
        let manual = ManualRoute {
            store_id: String::new(),
            payment_method_id: "btc-chain".into(),
            action: "provenance_only".into(),
            wallet: String::new(),
        };
        assert_eq!(
            validate(Some(&plan), &[], Some(&manual), &w),
            Some("manual_store_id")
        );
        let filled = ManualRoute {
            store_id: " STORE1 ".into(),
            ..manual
        };
        assert_eq!(validate(Some(&plan), &[], Some(&filled), &w), None);
        assert_eq!(
            build_create_routes(&plan, &[], Some(&filled), "Shop"),
            vec![
                json!({"store_id": "STORE1", "payment_method_id": "BTC-CHAIN", "action": "provenance_only", "network": "regtest"})
            ]
        );
    }

    #[test]
    fn detected_network_only_for_new_instance() {
        let plan = read_only_plan();
        let routes = vec![
            json!({"store_id": MAIN, "payment_method_id": "BTC-CHAIN", "action": "provenance_only"}),
        ];
        let inline = build_create_args(
            &Instance::New {
                backend_label: "Shop".into(),
                server_url: "https://pay.example".into(),
                api_key: "key".into(),
            },
            true,
            &plan,
            &routes,
            " Shop ",
        );
        assert_eq!(inline["mode"], "account");
        assert_eq!(inline["label"], "Shop");
        assert_eq!(inline["network"], "regtest");
        assert_eq!(inline["sync_provenance"], true);
        let saved = build_create_args(
            &Instance::Saved {
                backend: "shop-btcpay".into(),
            },
            false,
            &plan,
            &routes,
            "Shop",
        );
        assert!(saved.get("network").is_none());
    }

    #[test]
    fn refreshes_created_sources_and_each_mapped_wallet_once() {
        let routes = vec![
            json!({"store_id": POPUP, "payment_method_id": "BTC-CHAIN", "action": "existing_wallet", "wallet": "Shop Trezor"}),
            json!({"store_id": MAIN, "payment_method_id": "BTC-CHAIN", "action": "existing_wallet", "wallet": "Shop Trezor"}),
            json!({"store_id": CAFE, "payment_method_id": "BTC-LN", "action": "provenance_only"}),
        ];
        let (src, mapped) = wallets_to_refresh(&routes, &["Shop - Café - BTC-CHAIN".to_owned()]);
        assert_eq!(src, vec!["Shop - Café - BTC-CHAIN".to_owned()]);
        assert_eq!(mapped, vec!["Shop Trezor".to_owned()]);
    }

    #[test]
    fn settlement_wallets_on_matching_chain() {
        let w = wallets();
        let l = |v: Vec<WalletOption>| v.into_iter().map(|x| x.label).collect::<Vec<_>>();
        assert_eq!(
            l(wallet_options_for_method(Some("bitcoin"), "BTC-CHAIN", &w)),
            vec!["Shop Trezor"]
        );
        assert_eq!(
            l(wallet_options_for_method(None, "LBTC-CHAIN", &w)),
            vec!["Liquid Hot"]
        );
    }

    #[test]
    fn summarises_and_classifies() {
        let plan = read_only_plan();
        let w = wallets();
        let e = plan_routes(&plan, &default_route_choices(&plan, &w), &w);
        assert_eq!(summarize_routes(&e), [0, 2, 1, 1, 0]);
        assert_eq!(rail_kind("onchain", Some("liquid"), Some(true)), "liquid");
        assert_eq!(rail_kind("onchain", None, Some(false)), "other_asset");
        assert_eq!(rail_kind("plugin", None, Some(false)), "plugin");
        assert_eq!(key_risk_tone(Some("read_only")), Tone::Good);
        assert_eq!(key_risk_tone(Some("can_modify_store")), Tone::Warning);
        assert_eq!(key_risk_tone(Some("server_admin")), Tone::Danger);
        assert_eq!(key_risk_tone(None), Tone::Neutral);
    }

    #[test]
    fn store_freshness_states() {
        let base = StoreSyncState {
            last_attempt_at: Some("2026-01-02T00:00:00Z".into()),
            last_success_at: Some("2026-01-02T00:00:00Z".into()),
            age_seconds: Some(60.0),
            ..StoreSyncState::default()
        };
        assert_eq!(store_freshness(None), None);
        assert_eq!(store_freshness(Some(&base)), Some("fresh"));
        assert_eq!(
            store_freshness(Some(&StoreSyncState {
                stale: true,
                ..base.clone()
            })),
            Some("stale")
        );
        assert_eq!(
            store_freshness(Some(&StoreSyncState {
                stale: true,
                last_error_code: Some("auth_error".into()),
                ..base.clone()
            })),
            Some("failed")
        );
        assert_eq!(
            store_freshness(Some(&StoreSyncState {
                last_success_at: None,
                never_synced: true,
                stale: true,
                ..base
            })),
            Some("never")
        );
    }

    #[test]
    fn payment_ledger_only_for_bookable_rails() {
        let plan = read_only_plan();
        let w = wallets();
        assert_eq!(
            plan_routes(&plan, &HashMap::new(), &w)[1].choice.action,
            "payment_ledger"
        );
        let ln = Method {
            actions: None,
            ledger_group: None,
            recommendation: None,
            ..plan.payment_methods[1].clone()
        };
        let p2 = Plan {
            payment_methods: vec![ln],
            ..plan.clone()
        };
        let mut ch = HashMap::new();
        ch.insert(route_key(CAFE, "BTC-LN"), Choice::new("payment_ledger", ""));
        assert_eq!(
            plan_routes(&p2, &ch, &w)[0].choice.action,
            "provenance_only"
        );
    }

    #[test]
    fn recognises_server_urls() {
        assert!(looks_like_server_url("https://pay.example.com/stores/abc"));
        assert!(looks_like_server_url("http://127.0.0.1:18549"));
        assert!(!looks_like_server_url("pay.example.com"));
        assert!(!looks_like_server_url(""));
    }

    #[test]
    fn parses_a_daemon_plan() {
        let v = json!({"backend": "b", "stores": [{"id": "s", "name": "S", "capabilities": {"invoices": false, "payouts": null}}],
            "payment_methods": [{"store_id": "s", "payment_method_id": "BTC-CHAIN", "label": "L", "enabled": true, "sync_supported": true, "rail": "onchain"}]});
        let p: Plan = serde_json::from_value(v).expect("plan");
        assert_eq!(p.stores[0].capability("invoices"), Some(false));
        assert_eq!(p.stores[0].capability("payouts"), None);
        assert_eq!(p.payment_methods[0].rail, "onchain");
    }
}
