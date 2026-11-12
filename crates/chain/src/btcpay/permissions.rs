//! `kassiber/btcpay/permissions.py`: the Greenfield permission model and the
//! key setup guide. Pure; building a guide or authorize link performs no I/O.
use super::url::normalize_server_url;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const VIEW_STORE_SETTINGS: &str = "btcpay.store.canviewstoresettings";
pub const MODIFY_STORE_SETTINGS: &str = "btcpay.store.canmodifystoresettings";
pub const VIEW_WALLET: &str = "btcpay.store.canviewwallet";
pub const VIEW_INVOICES: &str = "btcpay.store.canviewinvoices";
pub const VIEW_PAYMENT_REQUESTS: &str = "btcpay.store.canviewpaymentrequests";
pub const VIEW_PAYOUTS: &str = "btcpay.store.canviewpayouts";
pub const VIEW_PULL_PAYMENTS: &str = "btcpay.store.canviewpullpayments";
pub const VIEW_REPORTS: &str = "btcpay.store.canviewreports";
pub const UNRESTRICTED: &str = "unrestricted";

/// Direct inclusions from BTCPay Server 2.3 and 2.4 (`GET /misc/permissions`;
/// 2.4 adds the Wallets plugin's policies under `canmodifystoresettings`).
const DIRECT_INCLUDES: &[(&str, &[&str])] = &[
    (
        UNRESTRICTED,
        &[
            "btcpay.server.canmodifyserversettings",
            "btcpay.server.canviewusers",
            "btcpay.impersonation.canimpersonate",
            MODIFY_STORE_SETTINGS,
            "btcpay.user.candeleteuser",
            "btcpay.user.canmanagenotificationsforuser",
            "btcpay.user.canmodifyprofile",
        ],
    ),
    (
        "btcpay.server.canmodifyserversettings",
        &[
            "btcpay.server.canmanageusers",
            "btcpay.server.canuseinternallightningnode",
        ],
    ),
    (
        "btcpay.server.canmanageusers",
        &["btcpay.server.cancreateuser"],
    ),
    (
        "btcpay.server.canuseinternallightningnode",
        &[
            "btcpay.server.cancreatelightninginvoiceinternalnode",
            "btcpay.server.canviewlightninginvoiceinternalnode",
        ],
    ),
    (
        MODIFY_STORE_SETTINGS,
        &[
            VIEW_STORE_SETTINGS,
            "btcpay.store.canmodifyinvoices",
            "btcpay.store.canmodifypaymentrequests",
            "btcpay.store.canmanagepullpayments",
            "btcpay.store.canmanagepayouts",
            "btcpay.store.canuselightningnode",
            "btcpay.store.canmodifyofferings",
            "btcpay.store.webhooks.canmodifywebhooks",
            "btcpay.store.canmanagewallets",
            "btcpay.store.canmanagelegacyaccesstokens",
            "btcpay.store.cansendstoreemails",
        ],
    ),
    (
        "btcpay.store.canmanagewallets",
        &[
            "btcpay.store.canmanagewalletsettings",
            "btcpay.store.canmanagewallettransactions",
        ],
    ),
    ("btcpay.store.canmanagewalletsettings", &[VIEW_WALLET]),
    (
        "btcpay.store.canmanagewallettransactions",
        &[
            "btcpay.store.cancreatetransactions",
            "btcpay.store.cansigntransactions",
            "btcpay.store.canbroadcasttransactions",
            "btcpay.store.cancanceltransactions",
        ],
    ),
    ("btcpay.store.cancreatetransactions", &[VIEW_WALLET]),
    ("btcpay.store.cansigntransactions", &[VIEW_WALLET]),
    ("btcpay.store.canbroadcasttransactions", &[VIEW_WALLET]),
    ("btcpay.store.cancanceltransactions", &[VIEW_WALLET]),
    (
        VIEW_STORE_SETTINGS,
        &[
            VIEW_INVOICES,
            VIEW_PAYMENT_REQUESTS,
            VIEW_PAYOUTS,
            VIEW_PULL_PAYMENTS,
            VIEW_REPORTS,
        ],
    ),
    (
        "btcpay.store.canmodifyinvoices",
        &[
            VIEW_INVOICES,
            "btcpay.store.cancreateinvoice",
            "btcpay.store.cancreatelightninginvoice",
            "btcpay.store.canmanageinvoicestatus",
        ],
    ),
    ("btcpay.store.canmanageinvoicestatus", &[VIEW_INVOICES]),
    (
        "btcpay.store.canmodifypaymentrequests",
        &[VIEW_PAYMENT_REQUESTS],
    ),
    (
        "btcpay.store.canmanagepullpayments",
        &[
            "btcpay.store.cancreatepullpayments",
            "btcpay.store.canarchivepullpayments",
        ],
    ),
    (
        "btcpay.store.cancreatepullpayments",
        &["btcpay.store.cancreatenonapprovedpullpayments"],
    ),
    (
        "btcpay.store.cancreatenonapprovedpullpayments",
        &[VIEW_PULL_PAYMENTS],
    ),
    ("btcpay.store.canmanagepayouts", &[VIEW_PAYOUTS]),
    (
        "btcpay.store.canuselightningnode",
        &["btcpay.store.cancreatelightninginvoice"],
    ),
    (
        "btcpay.store.cancreatelightninginvoice",
        &["btcpay.store.canviewlightninginvoice"],
    ),
    (
        "btcpay.store.canmodifyofferings",
        &[
            "btcpay.store.canviewofferings",
            "btcpay.store.canmanagesubscribers",
            "btcpay.store.cancreditsubscribers",
        ],
    ),
    (
        "btcpay.user.canmodifyprofile",
        &["btcpay.user.canviewprofile"],
    ),
    (
        "btcpay.user.canmanagenotificationsforuser",
        &["btcpay.user.canviewnotificationsforuser"],
    ),
];

pub const READ_ONLY_PERMISSIONS: &[&str] = &[
    VIEW_STORE_SETTINGS,
    VIEW_INVOICES,
    VIEW_PAYMENT_REQUESTS,
    VIEW_PAYOUTS,
    VIEW_PULL_PAYMENTS,
    VIEW_REPORTS,
    VIEW_WALLET,
    "btcpay.store.canviewofferings",
    "btcpay.store.canviewlightninginvoice",
    "btcpay.user.canviewprofile",
    "btcpay.user.canviewnotificationsforuser",
    "btcpay.server.canviewusers",
    "btcpay.server.canviewlightninginvoiceinternalnode",
];

/// Kassiber capability → the narrowest permission that grants it on BTCPay
/// Server 2.4 and later, where the Wallets plugin guards the on-chain
/// wallet, in order.
pub const CAPABILITY_PERMISSIONS: &[(&str, &str)] = &[
    ("store_catalog", VIEW_STORE_SETTINGS),
    ("wallet_preview", VIEW_WALLET),
    ("invoices", VIEW_INVOICES),
    ("payment_requests", VIEW_PAYMENT_REQUESTS),
    ("payouts", VIEW_PAYOUTS),
    ("pull_payments", VIEW_PULL_PAYMENTS),
    ("wallet_history", VIEW_WALLET),
];

/// The same on BTCPay Server 2.3 and earlier.
pub const LEGACY_CAPABILITY_PERMISSIONS: &[(&str, &str)] = &[
    ("store_catalog", VIEW_STORE_SETTINGS),
    ("wallet_preview", VIEW_STORE_SETTINGS),
    ("invoices", VIEW_INVOICES),
    ("payment_requests", VIEW_PAYMENT_REQUESTS),
    ("payouts", VIEW_PAYOUTS),
    ("pull_payments", VIEW_PULL_PAYMENTS),
    ("wallet_history", MODIFY_STORE_SETTINGS),
];

/// The first BTCPay version whose wallet endpoints need `canviewwallet`.
pub const WALLET_POLICY_VERSION: (i64, i64) = (2, 4);

/// `server_version(value)`: `(major, minor)` of a BTCPay version, or `None`.
/// Parts beyond `i64` saturate: only the comparison with
/// `WALLET_POLICY_VERSION` reads them.
pub fn server_version(value: &Value) -> Option<(i64, i64)> {
    let text = if kassiber_envelope::truthy(value) {
        kassiber_envelope::py_str(value)
    } else {
        String::new()
    };
    let text = kassiber_pytext::strip(&text).trim_start_matches('v');
    let mut parts = text.split('.');
    let int = |part: &str| {
        kassiber_pytext::parse_int(part).ok().map(|n| {
            i64::try_from(&n).unwrap_or(if n < kassiber_pyjson::BigInt::from(0) {
                i64::MIN
            } else {
                i64::MAX
            })
        })
    };
    let major = int(parts.next().unwrap_or_default())?;
    let minor = match parts.next() {
        Some(part) => int(part)?,
        None => 0,
    };
    Some((major, minor))
}

/// `capability_permissions(version)`: the table for a known version, both
/// (newest first) for an unknown one.
pub fn capability_permissions(
    version: Option<(i64, i64)>,
) -> Vec<&'static [(&'static str, &'static str)]> {
    match version {
        None => vec![CAPABILITY_PERMISSIONS, LEGACY_CAPABILITY_PERMISSIONS],
        Some(version) if version >= WALLET_POLICY_VERSION => vec![CAPABILITY_PERMISSIONS],
        Some(_) => vec![LEGACY_CAPABILITY_PERMISSIONS],
    }
}

/// `wallet_history_preset(version)`: the preset whose key reads wallet
/// history on this server.
pub fn wallet_history_preset(version: Option<(i64, i64)>) -> &'static str {
    match version {
        Some(version) if version >= WALLET_POLICY_VERSION => "read_only",
        _ => "wallet_history",
    }
}

pub struct Preset {
    pub id: &'static str,
    pub permissions: &'static [&'static str],
    pub label: &'static str,
    pub summary: &'static str,
    pub capabilities: &'static [&'static str],
    pub default_scope: &'static str,
}

pub const PRESETS: [Preset; 2] = [
    Preset {
        id: "read_only",
        permissions: &[VIEW_STORE_SETTINGS, VIEW_WALLET],
        label: "Read-only (recommended)",
        summary: "Invoices, payments, refunds and payouts, payment requests, and a wallet check that recognises which wallet each store pays into. On BTCPay Server 2.4 and later it also reads each store's on-chain wallet history. The key cannot change your store.",
        capabilities: &[
            "store_catalog",
            "wallet_preview",
            "invoices",
            "payment_requests",
            "payouts",
            "pull_payments",
        ],
        default_scope: "all",
    },
    Preset {
        id: "wallet_history",
        permissions: &[MODIFY_STORE_SETTINGS],
        label: "Include BTCPay wallet history",
        summary: "Adds BTCPay's own on-chain wallet history with its labels and comments on BTCPay Server 2.3 and earlier, which only offer it under the 'Modify your stores' permission. That permission could also change the store's payout wallet. On 2.4 and later the read-only key already includes wallet history.",
        capabilities: &[
            "store_catalog",
            "wallet_preview",
            "invoices",
            "payment_requests",
            "payouts",
            "pull_payments",
            "wallet_history",
        ],
        default_scope: "single",
    },
];

/// `preset_permissions(preset)`'s refusal for an unknown preset.
pub fn preset(id: &str) -> Result<&'static Preset, AppError> {
    PRESETS.iter().find(|p| p.id == id).ok_or_else(|| {
        AppError::new("validation", format!("Unknown BTCPay key preset '{id}'"))
            .with_hint("Choose one of: read_only, wallet_history.")
    })
}

/// `expand_permission`: the inclusion closure (a name includes itself).
pub fn expand_permission(name: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![name.to_owned()];
    while let Some(next) = stack.pop() {
        if seen.insert(next.clone()) {
            if let Some((_, children)) = DIRECT_INCLUDES.iter().find(|(k, _)| *k == next) {
                stack.extend(children.iter().map(|c| (*c).to_owned()));
            }
        }
    }
    seen
}

/// `permission_label`.
pub fn permission_label(name: &str) -> String {
    let base = name.split(':').next().unwrap_or_default();
    match base {
        VIEW_STORE_SETTINGS => "View your stores",
        MODIFY_STORE_SETTINGS => "Modify your stores",
        VIEW_WALLET => "View wallets",
        VIEW_INVOICES => "View invoices",
        VIEW_PAYMENT_REQUESTS => "View payment requests",
        VIEW_PAYOUTS => "View payouts",
        VIEW_PULL_PAYMENTS => "View pull payments",
        VIEW_REPORTS => "View reports",
        UNRESTRICTED => "Unrestricted access",
        other => other,
    }
    .to_owned()
}

/// `split_permission`.
pub fn split_permission(value: &str) -> (String, Option<String>) {
    let raw = kassiber_pytext::strip(value);
    if raw.starts_with("btcpay.") {
        if let Some((name, store)) = raw.split_once(':') {
            return (
                name.to_owned(),
                (!store.is_empty()).then(|| store.to_owned()),
            );
        }
    }
    (raw.to_owned(), None)
}

/// `KeyGrant`: the effective permissions of one API key.
#[derive(Clone, Debug, Default)]
pub struct KeyGrant {
    pub known: bool,
    pub global: BTreeSet<String>,
    pub stores: BTreeMap<String, BTreeSet<String>>,
    pub raw: Vec<String>,
}

impl KeyGrant {
    /// `parse_key_grant`; `None` is an unknown grant.
    pub fn parse(permissions: Option<&[String]>) -> Self {
        let Some(permissions) = permissions else {
            return Self::default();
        };
        let mut grant = Self {
            known: true,
            ..Self::default()
        };
        for value in permissions {
            let (name, store) = split_permission(value);
            if name.is_empty() {
                continue;
            }
            grant.raw.push(value.clone());
            match store {
                Some(store) => {
                    grant.stores.entry(store).or_default().insert(name);
                }
                None => {
                    grant.global.insert(name);
                }
            }
        }
        grant.raw.sort();
        grant
    }

    pub fn all_stores(&self) -> bool {
        self.global
            .iter()
            .any(|n| n.starts_with("btcpay.store.") || n == UNRESTRICTED)
    }

    pub fn effective(&self, store: Option<&str>) -> BTreeSet<String> {
        let mut granted = BTreeSet::new();
        for name in &self.global {
            granted.extend(expand_permission(name));
        }
        if let Some(store) = store {
            for name in self.stores.get(store).into_iter().flatten() {
                granted.extend(expand_permission(name));
            }
        }
        granted
    }

    pub fn has(&self, permission: &str, store: Option<&str>) -> bool {
        self.effective(store).contains(permission)
    }

    /// `capabilities(store_id, version)`, in capability order: what the key
    /// can read on a server of `version`; with an unknown version, a
    /// capability either table grants counts.
    pub fn capabilities(&self, store: Option<&str>, version: Option<(i64, i64)>) -> Value {
        let effective = self.effective(store);
        let tables = capability_permissions(version);
        dict(CAPABILITY_PERMISSIONS.iter().map(|(capability, _)| {
            let granted = tables.iter().any(|table| {
                table
                    .iter()
                    .any(|(c, permission)| c == capability && effective.contains(*permission))
            });
            (*capability, Value::Bool(granted))
        }))
    }

    fn names(&self) -> BTreeSet<String> {
        let mut names = self.global.clone();
        for values in self.stores.values() {
            names.extend(values.iter().cloned());
        }
        names
    }

    /// `key_risk`.
    pub fn risk(&self) -> &'static str {
        if !self.known {
            return "unknown";
        }
        let names = self.names();
        if names.contains(UNRESTRICTED)
            || names.iter().any(|n| {
                n.starts_with("btcpay.server.") && !READ_ONLY_PERMISSIONS.contains(&n.as_str())
            })
        {
            return "server_admin";
        }
        let mut expanded = BTreeSet::new();
        for name in &names {
            expanded.extend(expand_permission(name));
        }
        if expanded.contains(MODIFY_STORE_SETTINGS) {
            "can_modify_store"
        } else if expanded
            .iter()
            .any(|n| !READ_ONLY_PERMISSIONS.contains(&n.as_str()))
        {
            "can_write"
        } else {
            "read_only"
        }
    }

    /// `excess_permissions(grant, wallet_history=...)`, sorted.
    pub fn excess(&self, wallet_history: bool) -> Vec<String> {
        if !self.known {
            return Vec::new();
        }
        let mut allowed: BTreeSet<String> = READ_ONLY_PERMISSIONS
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        if wallet_history {
            allowed.extend(expand_permission(MODIFY_STORE_SETTINGS));
        }
        self.names()
            .into_iter()
            .filter(|n| !allowed.contains(n))
            .collect()
    }

    /// `grant_summary(grant, version)`.
    pub fn summary(&self, version: Option<(i64, i64)>) -> Value {
        let scope = if !self.known {
            "unknown"
        } else if self.all_stores() {
            "all_stores"
        } else if !self.stores.is_empty() {
            "selected_stores"
        } else {
            "no_stores"
        };
        let list = |items: Vec<String>| Value::List(items.into_iter().map(s).collect());
        dict([
            ("known", Value::Bool(self.known)),
            ("scope", s(scope)),
            ("store_ids", list(self.stores.keys().cloned().collect())),
            ("permissions", list(self.raw.clone())),
            ("risk", s(self.risk())),
            ("excess_permissions", list(self.excess(true))),
            (
                "global_capabilities",
                if self.known {
                    self.capabilities(None, version)
                } else {
                    dict::<&str>([])
                },
            ),
        ])
    }
}

/// `urlencode` (`quote_plus`, nothing safe beyond the unreserved set).
fn encode(raw: &str) -> String {
    let mut result = String::new();
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                result.push(b as char)
            }
            b' ' => result.push('+'),
            _ => result.push_str(&format!("%{b:02X}")),
        }
    }
    result
}

/// `build_authorize_url`: the base URL is normalized, the scope checked.
pub fn authorize_url(
    server_url: &str,
    preset_id: &str,
    store_ids: &[String],
    store_scope: Option<&str>,
) -> Result<String, AppError> {
    let base = normalize_server_url(server_url)?;
    let preset = preset(preset_id)?;
    let scope = store_scope.unwrap_or(preset.default_scope);
    if !["all", "single"].contains(&scope) {
        return Err(AppError::new(
            "validation",
            format!("Unknown BTCPay key store scope '{scope}'"),
        )
        .with_hint("Choose one of: all, single."));
    }
    let stores: Vec<&str> = store_ids
        .iter()
        .map(|s| kassiber_pytext::strip(s))
        .filter(|s| !s.is_empty())
        .collect();
    let mut query = vec![format!("applicationName={}", encode("Kassiber"))];
    for permission in preset.permissions {
        if stores.is_empty() {
            query.push(format!("permissions={}", encode(permission)));
        }
        for store in &stores {
            query.push(format!(
                "permissions={}",
                encode(&format!("{permission}:{store}"))
            ));
        }
    }
    query.push("strict=true".into());
    query.push(format!(
        "selectiveStores={}",
        if scope == "single" && stores.is_empty() {
            "true"
        } else {
            "false"
        }
    ));
    Ok(format!("{base}/api-keys/authorize?{}", query.join("&")))
}

/// `key_setup_guide(server_url, preset=..., store_scope=...)`.
pub fn setup_guide(
    server_url: Option<&str>,
    preset_id: &str,
    store_scope: Option<&str>,
) -> Result<Value, AppError> {
    let preset = preset(preset_id)?;
    let scope = store_scope.unwrap_or(preset.default_scope);
    let names = |permissions: &[&str]| Value::List(permissions.iter().map(|p| s(*p)).collect());
    let enabled = preset
        .permissions
        .iter()
        .map(|p| format!("{} ({p})", permission_label(p)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut guide = vec![
        ("preset", s(preset.id)),
        ("label", s(preset.label)),
        ("summary", s(preset.summary)),
        ("permissions", names(preset.permissions)),
        (
            "permission_labels",
            Value::List(preset.permissions.iter().map(|p| s(permission_label(p).as_str())).collect()),
        ),
        (
            "capabilities",
            Value::List(preset.capabilities.iter().map(|c| s(*c)).collect()),
        ),
        ("store_scope", s(scope)),
        (
            "manual_steps",
            Value::List(vec![
                s("Sign in to BTCPay Server with the account that owns the store."),
                s("Open Account → Manage Account → API Keys and choose Generate Key."),
                s(format!("Label the key 'Kassiber' and enable only: {enabled}.")),
                s(if scope == "all" {
                    "Leave the permission on all stores so one key serves every store, then generate the key."
                } else {
                    "Limit the permission to the store Kassiber should read, then generate the key."
                }),
                s("Copy the key into Kassiber. BTCPay shows it again on the API Keys page if you need it later."),
            ]),
        ),
        (
            "presets",
            Value::List(
                PRESETS
                    .iter()
                    .map(|p| {
                        dict([
                            ("id", s(p.id)),
                            ("label", s(p.label)),
                            ("permissions", names(p.permissions)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];
    if let Some(server_url) = server_url.filter(|u| !u.is_empty()) {
        guide.push((
            "authorize_url",
            s(super::url::to_py(&authorize_url(
                server_url,
                preset_id,
                &[],
                Some(scope),
            )?)),
        ));
    }
    Ok(dict(guide))
}
