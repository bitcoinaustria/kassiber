//! BTCPay store groups (Rust only; docs/reference/daemon.md#btcpay-store-groups):
//! which wallets each BTCPay store pays into, and through which payment
//! methods.
//!
//! A group is display structure. It reads the routes setup already stored
//! (BTCPay wallet sources and payment ledgers, `btcpay_provenance` mappings on
//! wallets the user tracks, and walletless invoice-only routes) and changes
//! nothing: membership never grants ownership, moves a balance, or decides a
//! tax fact, and each wallet keeps its own sync and pairing rules. A wallet
//! two stores pay into belongs to both groups and is still one wallet.
//!
//! BTCPay's store name is not stored on its own. Setup writes it into the
//! invoice-only route's label and into the generated name of the wallets it
//! creates (`<connection> - <store> - <method>`); the group takes it from
//! there, and is unnamed (the desktop shows the store id) when the user
//! renamed those wallets. This read never opens a socket.
use crate::database_error;
use kassiber_chain::btcpay::payment_methods as methods;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params, Connection};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};

/// How a store's payment method reaches a wallet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// The store pays into a wallet the user tracks with its own sync; BTCPay
    /// adds invoice provenance only.
    Mapped,
    /// Kassiber imports the store wallet's history from BTCPay.
    WalletSource,
    /// BTCPay's settled payments and payouts are the wallet's balance.
    PaymentLedger,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Mapped => "mapped",
            Role::WalletSource => "wallet_source",
            Role::PaymentLedger => "payment_ledger",
        }
    }
}

/// One wallet in one store's group.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub wallet_id: String,
    pub label: String,
    pub kind: String,
    pub role: Role,
    /// Canonical payment method ids, in rail order.
    pub payment_methods: Vec<String>,
    /// How many store groups this wallet is in (2+ when stores share it).
    pub store_count: usize,
}

/// One BTCPay store: the wallets it pays into and its invoice-only methods.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub backend: String,
    pub store_id: String,
    pub label: Option<String>,
    /// `route` (the invoice-only route's label) or `wallet_label` (parsed from
    /// a wallet name setup generated); `None` with no label.
    pub label_source: Option<&'static str>,
    pub wallets: Vec<Member>,
    /// Methods kept as invoice records only (`provenance_only`): no wallet
    /// here books them.
    pub records_only: Vec<String>,
}

impl Group {
    pub fn key(&self) -> String {
        format!("btcpay:{}:{}", self.backend, self.store_id)
    }

    fn to_value(&self) -> Value {
        dict([
            ("key", s(self.key())),
            ("kind", s("btcpay_store")),
            ("backend", s(self.backend.as_str())),
            ("store_id", s(self.store_id.as_str())),
            ("label", self.label.as_deref().map_or(Value::Null, s)),
            ("label_source", self.label_source.map_or(Value::Null, s)),
            (
                "wallets",
                Value::List(
                    self.wallets
                        .iter()
                        .map(|m| {
                            dict([
                                ("wallet_id", s(m.wallet_id.as_str())),
                                ("label", s(m.label.as_str())),
                                ("kind", s(m.kind.as_str())),
                                ("role", s(m.role.as_str())),
                                ("payment_methods", method_list(&m.payment_methods)),
                                ("store_count", Value::from(m.store_count as i64)),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("records_only", method_list(&self.records_only)),
        ])
    }
}

fn method_list(ids: &[String]) -> Value {
    Value::List(
        ids.iter()
            .map(|id| {
                let c = methods::classify(id);
                dict([
                    ("id", s(c.payment_method_id.as_str())),
                    ("rail", s(c.rail)),
                    ("settlement", s(c.settlement)),
                    ("label", s(c.label.as_str())),
                ])
            })
            .collect(),
    )
}

/// Rail order inside a group: on-chain, Liquid, Lightning, LNURL, Bark, other plugins.
fn method_rank(id: &str) -> (u8, String) {
    let rank = match methods::classify(id).settlement {
        "btc_onchain" => 0,
        "liquid_onchain" => 1,
        "lightning" => 2,
        "lnurl" => 3,
        "bark" => 4,
        "plugin" => 5,
        _ => 6,
    };
    (rank, id.to_owned())
}

fn sort_methods(ids: &mut Vec<String>) {
    ids.sort_by_key(|id| method_rank(id));
    ids.dedup();
}

fn text(config: &Json, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

fn canonical(raw: &str) -> Option<String> {
    let id = methods::canonical(raw);
    (!id.is_empty()).then_some(id)
}

/// The store part of a wallet name setup generated: `<connection> - <store> -
/// <suffix>`, where the suffix is a method id or its rail label. `None` for any
/// other name (the user renamed it, or a store name setup never wrote).
pub fn store_name_from_label(label: &str, methods_in: &[String]) -> Option<String> {
    let mut suffixes: Vec<String> = Vec::new();
    for id in methods_in {
        let c = methods::classify(id);
        suffixes.push(c.payment_method_id.clone());
        suffixes.push(c.label.clone());
        if c.ledger_group.as_deref() == Some("lightning") {
            suffixes.push("Lightning".into());
        }
    }
    let head = suffixes
        .iter()
        .find_map(|suffix| label.strip_suffix(&format!(" - {suffix}")))?;
    let (_, store) = head.split_once(" - ")?;
    let store = store.trim();
    (!store.is_empty()).then(|| store.to_owned())
}

type Key = (String, String);

#[derive(Default)]
struct Draft {
    members: BTreeMap<(String, Role), Member>,
    records_only: Vec<String>,
    route_label: Option<String>,
    wallet_labels: BTreeMap<String, usize>,
}

fn has_table(conn: &Connection, name: &str) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        params![name],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(database_error)
}

/// Every BTCPay store group of the profile, by store name.
pub fn groups(conn: &Connection, profile: &str) -> Result<Vec<Group>, AppError> {
    let mut drafts: BTreeMap<Key, Draft> = BTreeMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT id, label, kind, config_json FROM wallets WHERE profile_id = ? \
             ORDER BY label ASC, id ASC",
        )
        .map_err(database_error)?;
    let wallets = stmt
        .query_map(params![profile], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    for (id, label, kind, raw) in wallets {
        let Some(config) = raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Json>(raw).ok())
            .filter(Json::is_object)
        else {
            continue;
        };
        let add = |drafts: &mut BTreeMap<Key, Draft>,
                   backend: String,
                   store: String,
                   role: Role,
                   ids: Vec<String>| {
            let draft = drafts.entry((backend, store)).or_default();
            let member = draft
                .members
                .entry((id.clone(), role))
                .or_insert_with(|| Member {
                    wallet_id: id.clone(),
                    label: label.clone(),
                    kind: kind.clone(),
                    role,
                    payment_methods: Vec::new(),
                    store_count: 0,
                });
            member.payment_methods.extend(ids);
            sort_methods(&mut member.payment_methods);
        };
        // A wallet setup created: a BTCPay wallet source or a payment ledger.
        let store = text(&config, "store_id");
        let backend = text(&config, "backend").map(|b| b.to_lowercase());
        let btcpay = text(&config, "sync_source").as_deref() == Some("btcpay")
            || text(&config, "sync_mode").as_deref() == Some("btcpay");
        if kind == "custom" && btcpay {
            if let (Some(backend), Some(store)) = (backend.clone(), store) {
                let ledger = text(&config, "source_mode").as_deref() == Some("payments");
                let mut ids: Vec<String> = config
                    .get("payment_method_ids")
                    .and_then(Json::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Json::as_str)
                    .filter_map(canonical)
                    .collect();
                if ids.is_empty() {
                    ids.extend(text(&config, "payment_method_id").and_then(|m| canonical(&m)));
                }
                let role = if ledger {
                    Role::PaymentLedger
                } else {
                    Role::WalletSource
                };
                if let Some(name) = store_name_from_label(&label, &ids) {
                    *drafts
                        .entry((backend.clone(), store.clone()))
                        .or_default()
                        .wallet_labels
                        .entry(name)
                        .or_default() += 1;
                }
                add(&mut drafts, backend, store, role, ids);
            }
        }
        // A wallet the user tracks, which BTCPay stores pay into.
        for route in config
            .get("btcpay_provenance")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(backend), Some(store)) = (text(route, "backend"), text(route, "store_id"))
            else {
                continue;
            };
            let method = text(route, "payment_method_id").unwrap_or_else(|| "BTC-CHAIN".into());
            add(
                &mut drafts,
                backend.to_lowercase(),
                store,
                Role::Mapped,
                canonical(&method).into_iter().collect(),
            );
        }
    }
    if has_table(conn, "btcpay_account_routes")? {
        let mut stmt = conn
            .prepare(
                "SELECT backend_name, store_id, payment_method_id, label \
                 FROM btcpay_account_routes \
                 WHERE profile_id = ? AND action = 'provenance_only' \
                 ORDER BY backend_name, store_id, payment_method_id",
            )
            .map_err(database_error)?;
        let routes = stmt
            .query_map(params![profile], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        for (backend, store, method, label) in routes {
            let draft = drafts.entry((backend.to_lowercase(), store)).or_default();
            draft.records_only.extend(canonical(&method));
            if draft.route_label.is_none() {
                draft.route_label = label.map(|l| l.trim().to_owned()).filter(|l| !l.is_empty());
            }
        }
    }
    let mut counts: BTreeMap<String, BTreeSet<Key>> = BTreeMap::new();
    for (key, draft) in &drafts {
        for (wallet, _) in draft.members.keys() {
            counts
                .entry(wallet.clone())
                .or_default()
                .insert(key.clone());
        }
    }
    let mut out: Vec<Group> = drafts
        .into_iter()
        .map(|((backend, store_id), mut draft)| {
            // A name two generated wallets agree on wins; ties go to the first by name.
            let parsed = draft
                .wallet_labels
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(name, _)| name.clone());
            let (label, label_source) = match (draft.route_label.take(), parsed) {
                (Some(l), _) => (Some(l), Some("route")),
                (None, Some(l)) => (Some(l), Some("wallet_label")),
                (None, None) => (None, None),
            };
            let mut wallets: Vec<Member> = draft
                .members
                .into_values()
                .map(|mut m| {
                    m.store_count = counts.get(&m.wallet_id).map_or(1, BTreeSet::len);
                    m
                })
                .collect();
            // Wallets in rail order of their first method, then by name.
            wallets.sort_by(|a, b| {
                let first = |m: &Member| {
                    m.payment_methods
                        .first()
                        .map_or((u8::MAX, String::new()), |id| method_rank(id))
                };
                first(a)
                    .cmp(&first(b))
                    .then_with(|| a.role.cmp(&b.role))
                    .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            });
            // A method a wallet books is not also "records only".
            let booked: BTreeSet<&String> =
                wallets.iter().flat_map(|m| &m.payment_methods).collect();
            draft.records_only.retain(|m| !booked.contains(m));
            sort_methods(&mut draft.records_only);
            Group {
                backend,
                store_id,
                label,
                label_source,
                wallets,
                records_only: draft.records_only,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        let name = |g: &Group| g.label.as_deref().map(str::to_lowercase);
        // Named stores first, by name; unnamed ones by id.
        name(a)
            .is_none()
            .cmp(&name(b).is_none())
            .then_with(|| name(a).cmp(&name(b)))
            .then_with(|| a.backend.cmp(&b.backend))
            .then_with(|| a.store_id.cmp(&b.store_id))
    });
    Ok(out)
}

/// `ui.sources.groups`: every BTCPay store group and a summary.
pub fn report(conn: &Connection, profile: &str) -> Result<Value, AppError> {
    let groups = groups(conn, profile)?;
    let backends: BTreeSet<&str> = groups.iter().map(|g| g.backend.as_str()).collect();
    let wallets: BTreeSet<&str> = groups
        .iter()
        .flat_map(|g| g.wallets.iter().map(|m| m.wallet_id.as_str()))
        .collect();
    Ok(dict([
        (
            "summary",
            dict([
                ("groups", Value::from(groups.len() as i64)),
                ("backends", Value::from(backends.len() as i64)),
                ("wallets", Value::from(wallets.len() as i64)),
            ]),
        ),
        (
            "groups",
            Value::List(groups.iter().map(Group::to_value).collect()),
        ),
    ]))
}

#[cfg(test)]
mod tests;
