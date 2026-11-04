//! Address / transaction-id ownership reconciliation (`core.ownership`).
//!
//! Decides whether pasted addresses, txids, invoices and silent-payment
//! addresses belong to the active profile's wallets, from local evidence
//! only: the watch-only output inventory, exact stored receive outpoints,
//! local transaction graphs, address lists and offline derivation of active
//! and retired policies. Nothing here contacts a network; on-chain lookups are
//! the separate opt-in tier, whose results are classified by `classify_txid`.
//! A match is ownership evidence for display; it grants no custody, basis or
//! tax fact.
mod codec;
pub mod onchain;
mod silent;

use crate::{database_error, descriptors};
use kassiber_envelope::{dict, py_str, s, truthy, AppError};
use kassiber_pyjson::{BigInt, Value};
use kassiber_store::rusqlite::{types::ValueRef, Connection};
use serde_json::Value as Json;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

pub use codec::{address_script, decode_invoice, display_invoice};

type Result<T> = std::result::Result<T, AppError>;

/// Per-branch derivation ceiling for an interactive run.
pub const DEFAULT_SCAN_TO_INDEX: i64 = 500;
/// Hard ceiling so an absurd depth cannot wedge the daemon.
pub const MAX_SCAN_TO_INDEX: i64 = 20_000;
/// Tokens harvested from one CSV before the harvest stops.
pub const MAX_HARVEST_CANDIDATES: usize = 10_000;

const CASE_INSENSITIVE: [&str; 9] = [
    "bc1", "tb1", "bcrt1", "lq1", "tlq1", "ex1", "tex1", "el1", "ert1",
];
const PRIVATE_KEY_NOTE: &str =
    "Private keys are never used. Kassiber is watch-only and kept nothing of this entry.";

/// What the caller asked to identify (`_identify_inputs`).
#[derive(Default)]
pub struct Inputs {
    pub addresses: Vec<String>,
    pub txids: Vec<String>,
    pub candidates: Vec<String>,
    pub text: Option<String>,
    pub csv_text: Option<String>,
    pub scan_to_index: i64,
}

impl Inputs {
    /// `_identify_inputs` for an active profile. List fields take strings and
    /// integers; other members are dropped like Python's filter.
    pub fn from_args(args: Option<&Value>) -> Result<Self> {
        let invalid = |message: &str| AppError::new("validation", message);
        let args = match args {
            None | Some(Value::Null) => {
                return Ok(Self {
                    scan_to_index: DEFAULT_SCAN_TO_INDEX,
                    ..Self::default()
                })
            }
            Some(args @ Value::Dict(_)) => args,
            Some(_) => return Err(invalid("ui.wallets.identify args must be an object")),
        };
        let list = |key| -> Result<Vec<String>> {
            match field(args, key) {
                Value::Null => Ok(vec![]),
                value @ Value::Str(_) => Ok(vec![py_str(value)]),
                Value::List(items) => Ok(items
                    .iter()
                    .filter(|item| matches!(item, Value::Str(_) | Value::Int(_) | Value::Bool(_)))
                    .map(py_str)
                    .collect()),
                _ => Err(invalid(
                    "ui.wallets.identify list fields must be arrays of strings",
                )),
            }
        };
        let text = |key| match field(args, key) {
            value @ Value::Str(_) => Some(py_str(value)),
            _ => None,
        };
        let scan_to_index = match field(args, "scan_to_index") {
            Value::Int(number) => number.clone(),
            Value::Bool(flag) => BigInt::from(u8::from(*flag)),
            _ => BigInt::from(DEFAULT_SCAN_TO_INDEX),
        };
        Ok(Self {
            addresses: list("addresses")?,
            txids: list("txids")?,
            candidates: list("candidates")?,
            text: text("text"),
            csv_text: text("csv_text"),
            scan_to_index: i64::try_from(
                scan_to_index.clamp(BigInt::from(-1), BigInt::from(MAX_SCAN_TO_INDEX)),
            )
            .expect("clamped depth"),
        })
    }
}

/// One way an owned address/script was reached (`OwnedMatch`).
#[derive(Clone)]
struct Match {
    wallet_id: String,
    wallet_label: String,
    account: String,
    chain: String,
    network: String,
    branch_label: String,
    address_index: Value,
    derivation_path: Value,
    source: &'static str,
    wallet_kind: String,
}

impl Match {
    fn sort_key(&self) -> (u8, u8, &str, &str) {
        let kind = u8::from(
            !["descriptor", "xpub", "silent-payment"].contains(&self.wallet_kind.as_str()),
        );
        let source = match self.source {
            "derived" => 0,
            "inventory" => 1,
            "address_list" => 2,
            "derived_history" => 3,
            "address_history" => 4,
            _ => 9,
        };
        (kind, source, &self.wallet_label, &self.wallet_id)
    }
    fn to_value(&self) -> Value {
        dict([
            ("wallet", s(self.wallet_label.as_str())),
            ("wallet_id", s(self.wallet_id.as_str())),
            ("wallet_kind", s(self.wallet_kind.as_str())),
            ("account", s(self.account.as_str())),
            ("chain", s(self.chain.as_str())),
            ("network", s(self.network.as_str())),
            ("branch", s(self.branch_label.as_str())),
            ("address_index", self.address_index.clone()),
            ("derivation_path", self.derivation_path.clone()),
            ("match_source", s(self.source)),
        ])
    }
}

fn sorted(matches: &[Match]) -> Vec<Match> {
    let mut matches = matches.to_vec();
    matches.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    matches
}

fn ambiguous(matches: &[Match]) -> bool {
    matches
        .iter()
        .map(|m| &m.wallet_id)
        .collect::<BTreeSet<_>>()
        .len()
        > 1
}

fn labels(matches: &[Match]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    sorted(matches)
        .into_iter()
        .filter(|m| seen.insert(m.wallet_label.clone()))
        .map(|m| m.wallet_label)
        .collect()
}

/// The in-memory ownership index (`OwnedIndex`).
#[derive(Default)]
pub struct Index {
    by_script: HashMap<String, Vec<Match>>,
    by_address: HashMap<String, Vec<Match>>,
    by_outpoint: HashMap<String, Vec<Match>>,
    by_scoped_outpoint: HashMap<(String, String, String), Vec<Match>>,
    txid_wallets: HashMap<String, BTreeSet<(String, String)>>,
    txid_wallets_by_scope: HashMap<(String, String, String), BTreeSet<(String, String)>>,
}

impl Index {
    fn add_script(&mut self, script: Option<String>, item: &Match) {
        if let Some(script) = script.filter(|s| !s.is_empty()) {
            self.by_script
                .entry(script.to_lowercase())
                .or_default()
                .push(item.clone());
        }
    }
    fn add_address(&mut self, address: &str, item: &Match) {
        let key = address_key(address);
        if !key.is_empty() {
            self.by_address.entry(key).or_default().push(item.clone());
        }
    }
    fn add_outpoint(&mut self, txid: &str, vout: &BigInt, item: &Match) {
        let outpoint = format!("{}:{vout}", txid.to_lowercase());
        self.by_outpoint
            .entry(outpoint.clone())
            .or_default()
            .push(item.clone());
        let (chain, network) = chain_network(&s(item.chain.as_str()), &s(item.network.as_str()));
        self.by_scoped_outpoint
            .entry((chain, network, outpoint))
            .or_default()
            .push(item.clone());
    }
    fn note_txid(&mut self, txid: &str, item: &Match) {
        let key = txid.to_lowercase();
        let wallet = (item.wallet_id.clone(), item.wallet_label.clone());
        self.txid_wallets
            .entry(key.clone())
            .or_default()
            .insert(wallet.clone());
        let (chain, network) = chain_network(&s(item.chain.as_str()), &s(item.network.as_str()));
        self.txid_wallets_by_scope
            .entry((chain, network, key))
            .or_default()
            .insert(wallet);
    }
    fn lookup_address(&self, address: &str) -> &[Match] {
        self.by_address
            .get(&address_key(address))
            .map_or(&[], Vec::as_slice)
    }
    fn lookup_script(&self, script: Option<&str>) -> &[Match] {
        script
            .filter(|s| !s.is_empty())
            .and_then(|s| self.by_script.get(&s.to_lowercase()))
            .map_or(&[], Vec::as_slice)
    }
    fn lookup_outpoint(&self, outpoint: &str, scope: Option<&(String, String)>) -> Vec<Match> {
        let key = outpoint.to_lowercase();
        if key.is_empty() {
            return vec![];
        }
        let matches = match scope {
            None => self.by_outpoint.get(&key),
            Some(scope) => {
                let (chain, network) = chain_network(&s(scope.0.as_str()), &s(scope.1.as_str()));
                self.by_scoped_outpoint.get(&(chain, network, key))
            }
        };
        matches.map(|matches| sorted(matches)).unwrap_or_default()
    }
    fn lookup_txid_wallets(
        &self,
        txid: &str,
        scope: Option<&(String, String)>,
    ) -> BTreeSet<(String, String)> {
        let key = txid.to_lowercase();
        if key.is_empty() {
            return BTreeSet::new();
        }
        match scope {
            None => self.txid_wallets.get(&key).cloned().unwrap_or_default(),
            Some(scope) => {
                let (chain, network) = chain_network(&s(scope.0.as_str()), &s(scope.1.as_str()));
                self.txid_wallets_by_scope
                    .get(&(chain, network, key))
                    .cloned()
                    .unwrap_or_default()
            }
        }
    }
}

fn address_key(address: &str) -> String {
    let text = kassiber_pytext::strip(address);
    let lower = text.to_lowercase();
    if CASE_INSENSITIVE.iter().any(|p| lower.starts_with(p)) {
        lower
    } else {
        text.to_owned()
    }
}

/// `str(value or "")`.
fn or_text(value: &Value) -> String {
    if truthy(value) {
        py_str(value)
    } else {
        String::new()
    }
}

/// `_index_chain_network`: blank legacy metadata means Bitcoin mainnet.
fn chain_network(chain: &Value, network: &Value) -> (String, String) {
    let chain = if truthy(chain) {
        py_str(chain)
    } else {
        "bitcoin".into()
    };
    let network = or_text(network);
    descriptors::normalize_chain(&chain)
        .and_then(|c| descriptors::normalize_network(&c, &network).map(|n| (c, n)))
        .unwrap_or_else(|_| {
            (
                kassiber_pytext::strip(&chain).to_lowercase(),
                kassiber_pytext::strip(&network).to_lowercase(),
            )
        })
}

/// `_normalize_chain_network_scope`.
fn match_scope(chain: &str, network: &str) -> Option<(String, String)> {
    let chain = kassiber_pytext::strip(chain);
    let network = kassiber_pytext::strip(network);
    if chain.is_empty() && network.is_empty() {
        return None;
    }
    let canonical = descriptors::normalize_chain(if chain.is_empty() { "bitcoin" } else { chain })
        .and_then(|c| {
            let n = if network.is_empty() {
                String::new()
            } else {
                descriptors::normalize_network(&c, network)?
            };
            Ok((c, n))
        });
    Some(canonical.unwrap_or_else(|_| (chain.to_lowercase(), network.to_lowercase())))
}

fn in_scope(matches: &[Match], scope: Option<&(String, String)>) -> Vec<Match> {
    let ordered = sorted(matches);
    let Some((chain, network)) = scope else {
        return ordered;
    };
    ordered
        .into_iter()
        .filter(|m| {
            match_scope(&m.chain, &m.network)
                .is_some_and(|(c, n)| &c == chain && (network.is_empty() || &n == network))
        })
        .collect()
}

/// Python's `int(value)` where TypeError/ValueError become `None`. An
/// infinite float is OverflowError there, which this refuses.
fn py_int(value: &Value) -> Result<Option<BigInt>> {
    Ok(match value {
        Value::Bool(flag) => Some(BigInt::from(u8::from(*flag))),
        Value::Int(number) => Some(number.clone()),
        Value::Float(number) if number.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Float(number) if number.is_nan() => None,
        Value::Float(number) => kassiber_pytext::parse_int(&format!("{:.0}", number.trunc())).ok(),
        Value::Str(text) => text
            .as_str()
            .and_then(|t| kassiber_pytext::parse_int(t).ok()),
        _ => None,
    })
}

fn sql_value(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::from(n),
        ValueRef::Real(n) => Value::Float(n),
        ValueRef::Text(t) => s(String::from_utf8_lossy(t).as_ref()),
        ValueRef::Blob(b) => Value::Bytes(b.to_vec()),
    }
}

fn rows(conn: &Connection, sql: &str, params: &[&str]) -> Result<Vec<Vec<Value>>> {
    let mut statement = conn.prepare(sql).map_err(database_error)?;
    let columns = statement.column_count();
    let mapped = statement
        .query_map(kassiber_store::rusqlite::params_from_iter(params), |row| {
            (0..columns)
                .map(|i| row.get_ref(i).map(sql_value))
                .collect()
        })
        .map_err(database_error)?;
    mapped
        .collect::<std::result::Result<_, _>>()
        .map_err(database_error)
}

/// `_wallet_config`: unreadable JSON is an empty config; a non-object is
/// refused like Python's failing `.get`.
fn wallet_config(raw: &Value) -> Result<Value> {
    let parsed = match raw {
        Value::Str(text) if truthy(raw) => text
            .as_str()
            .and_then(|t| kassiber_pytext::loads(t).ok())
            .unwrap_or_else(|| dict::<&str>([])),
        _ => dict::<&str>([]),
    };
    match parsed {
        Value::Dict(_) => Ok(parsed),
        other => Err(AppError::new(
            "internal_error",
            format!(
                "'{}' object has no attribute 'get'",
                kassiber_envelope::type_name(&other)
            ),
        )),
    }
}

fn field<'a>(config: &'a Value, key: &str) -> &'a Value {
    kassiber_envelope::get(config, key).unwrap_or(&Value::Null)
}

/// `wallets.normalize_addresses`.
fn normalize_addresses(value: &Value) -> Result<Vec<String>> {
    let values: Vec<Value> = match value {
        Value::Null => return Ok(vec![]),
        Value::Str(_) => py_str(value).split(',').map(s).collect(),
        Value::List(items) => items.clone(),
        Value::Dict(items) => items
            .iter()
            .map(|(key, _)| match key {
                kassiber_pyjson::Key::Str(key) => Value::Str(key.clone()),
                _ => Value::Null,
            })
            .collect(),
        other => {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "'{}' object is not iterable",
                    kassiber_envelope::type_name(other)
                ),
            ))
        }
    };
    let mut seen = BTreeSet::new();
    Ok(values
        .iter()
        .filter_map(|value| {
            let text = py_str(value);
            let address = kassiber_pytext::strip(&text);
            if address.is_empty() {
                return None;
            }
            let key = address_script(address)
                .map(|script| format!("script:{script}"))
                .unwrap_or_else(|| format!("text:{address}"));
            seen.insert(key).then(|| address.to_owned())
        })
        .collect())
}

/// `wallets.has_descriptor_sync_material`.
fn has_descriptor(config: &Value) -> bool {
    let present = |key| {
        let value = field(config, key);
        value != &Value::Null && !kassiber_pytext::strip(&py_str(value)).is_empty()
    };
    present("descriptor") || (present("xpub") && truthy(field(config, "script_types")))
}

struct Wallet {
    id: String,
    label: String,
    kind: String,
    account: String,
    config: Value,
}

/// `load_profile_wallets` with account labels, ordered by label.
fn load_wallets(conn: &Connection, profile_id: &str) -> Result<Vec<Wallet>> {
    rows(
        conn,
        "SELECT w.id, w.label, w.kind, w.config_json, a.code AS account_code, a.label AS account_label \
         FROM wallets w LEFT JOIN accounts a ON a.id = w.account_id WHERE w.profile_id = ? ORDER BY w.label ASC",
        &[profile_id],
    )?
    .into_iter()
    .map(|row| {
        let account = if truthy(&row[4]) { &row[4] } else { &row[5] };
        Ok(Wallet {
            id: py_str(&row[0]),
            label: py_str(&row[1]),
            kind: or_text(&row[2]),
            account: or_text(account),
            config: row[3].clone(),
        })
    })
    .collect()
}

/// `build_owned_index` with derivation enabled; returns non-fatal warnings.
fn build_index(
    conn: &Connection,
    profile_id: &str,
    wallets: &[Wallet],
    scan_to_index: i64,
) -> Result<(Index, Vec<String>)> {
    let mut index = Index::default();
    let mut warnings = vec![];
    let ids: BTreeSet<&str> = wallets.iter().map(|w| w.id.as_str()).collect();
    let mut summaries = HashMap::new();
    for row in rows(
        conn,
        "SELECT id, label, kind, config_json FROM wallets WHERE profile_id = ?",
        &[profile_id],
    )? {
        let config = wallet_config(&row[3])?;
        summaries.insert(
            py_str(&row[0]),
            (
                py_str(&row[1]),
                or_text(&row[2]),
                or_text(field(&config, "chain")),
                or_text(field(&config, "network")),
            ),
        );
    }
    let mut highest: HashMap<String, Vec<(String, BigInt)>> = HashMap::new();
    for row in rows(
        conn,
        "SELECT wallet_id, txid, vout, address, script_pubkey, branch_label, branch_index, \
         address_index, chain, network FROM wallet_utxos WHERE profile_id = ?",
        &[profile_id],
    )? {
        let wallet_id = py_str(&row[0]);
        if !ids.is_empty() && !ids.contains(wallet_id.as_str()) {
            continue;
        }
        let (label, kind, chain, network) = summaries.get(&wallet_id).cloned().unwrap_or_default();
        let label = if label.is_empty() {
            wallet_id.clone()
        } else {
            label
        };
        let chain = if truthy(&row[8]) {
            row[8].clone()
        } else {
            s(chain)
        };
        let network = if truthy(&row[9]) {
            row[9].clone()
        } else {
            s(network)
        };
        let (chain, network) = chain_network(&chain, &network);
        let item = Match {
            wallet_id: wallet_id.clone(),
            wallet_label: label,
            account: String::new(),
            chain,
            network,
            branch_label: or_text(&row[5]),
            address_index: row[7].clone(),
            derivation_path: Value::Null,
            source: "inventory",
            wallet_kind: kind,
        };
        index.add_address(&or_text(&row[3]), &item);
        let script = match &row[3] {
            Value::Str(_) if truthy(&row[3]) => address_script(&py_str(&row[3])),
            _ => None,
        }
        .or_else(|| normalized_script(&row[4]));
        index.add_script(script, &item);
        if truthy(&row[1]) && row[2] != Value::Null {
            let vout = py_int(&row[2])?.ok_or_else(|| {
                AppError::new("internal_error", "invalid literal for int() with base 10")
            })?;
            index.add_outpoint(&py_str(&row[1]), &vout, &item);
        }
        if truthy(&row[1]) {
            index.note_txid(&py_str(&row[1]), &item);
        }
        if let Value::Int(position) = &row[7] {
            if !item.branch_label.is_empty() && *position >= BigInt::from(0) {
                let branches = highest.entry(wallet_id).or_default();
                match branches
                    .iter_mut()
                    .find(|(branch, _)| branch == &item.branch_label)
                {
                    Some((_, value)) => *value = value.clone().max(position.clone()),
                    None => branches.push((item.branch_label.clone(), position.clone())),
                }
            }
        }
    }
    // One derivation per canonical descriptor and ceiling, shared by the
    // current and retired policies of every wallet.
    let mut derived: HashMap<(String, u32), std::result::Result<Vec<Json>, String>> =
        HashMap::new();
    for wallet in wallets {
        let config = wallet_config(&wallet.config)?;
        let configured = if truthy(field(&config, "ownership_scan_to_index")) {
            py_int(field(&config, "ownership_scan_to_index"))?
        } else {
            Some(BigInt::from(0))
        };
        if configured.is_none() {
            warnings.push(format!(
                "Wallet '{}': ownership scan depth is invalid",
                wallet.label
            ));
        }
        let scan = configured
            .unwrap_or_default()
            .max(BigInt::from(scan_to_index))
            .min(BigInt::from(MAX_SCAN_TO_INDEX));
        let mut configs = vec![(config, false)];
        for row in rows(
            conn,
            "SELECT private_material_json FROM wallet_policy_epochs WHERE wallet_id = ? AND status = 'retired' \
             ORDER BY created_at ASC, id ASC",
            &[&wallet.id],
        )? {
            let historic = match &row[0] {
                Value::Str(text) => text.as_str().and_then(|t| kassiber_pytext::loads(t).ok()),
                Value::Null => Some(dict::<&str>([])),
                _ => None,
            };
            if let Some(historic @ Value::Dict(_)) = historic {
                if truthy(&historic) {
                    configs.push((historic, true));
                }
            }
        }
        for (config, history) in configs {
            let (chain, network) =
                chain_network(field(&config, "chain"), field(&config, "network"));
            let item = Match {
                wallet_id: wallet.id.clone(),
                wallet_label: wallet.label.clone(),
                account: wallet.account.clone(),
                chain,
                network,
                branch_label: "address".into(),
                address_index: Value::Null,
                derivation_path: Value::Null,
                source: if history {
                    "address_history"
                } else {
                    "address_list"
                },
                wallet_kind: wallet.kind.clone(),
            };
            for address in normalize_addresses(field(&config, "addresses"))? {
                index.add_address(&address, &item);
                index.add_script(address_script(&address), &item);
            }
            if !has_descriptor(&config) {
                continue;
            }
            let floor = if truthy(field(&config, "scan_to_index")) {
                py_int(field(&config, "scan_to_index"))?
            } else {
                Some(BigInt::from(0))
            };
            let Some(floor) = floor else {
                warnings.push(format!(
                    "Wallet '{}': historic scan floor is invalid",
                    wallet.label
                ));
                continue;
            };
            let not_scanned =
                |code: &str| format!("Wallet '{}': descriptor not scanned ({code})", wallet.label);
            let plan = match descriptors::load(&kassiber_custody::codec::to_json(&config)) {
                Ok(Some(plan)) => plan,
                Ok(None) => continue,
                Err(error) => {
                    warnings.push(not_scanned(&error.into_app().code));
                    continue;
                }
            };
            let used = highest
                .get(&wallet.id)
                .and_then(|branches| branches.iter().map(|(_, n)| n).max().cloned())
                .unwrap_or_else(|| BigInt::from(-1));
            let ceiling = (scan.clone().max(floor) + BigInt::from(1))
                .max(used + BigInt::from(plan.gap_limit) + BigInt::from(1))
                .clamp(BigInt::from(1), BigInt::from(MAX_SCAN_TO_INDEX + 1));
            let ceiling = u32::try_from(ceiling).expect("clamped ceiling");
            let targets = derived
                .entry((plan.fingerprint.clone(), ceiling))
                .or_insert_with(|| plan.derive(None, 0, ceiling).map_err(|e| e.into_app().code));
            let targets = match targets {
                Ok(targets) => targets,
                Err(code) => {
                    warnings.push(not_scanned(code));
                    continue;
                }
            };
            for target in targets.iter() {
                let item = Match {
                    chain: plan.chain.clone(),
                    network: plan.network.clone(),
                    branch_label: target["branch_label"].as_str().unwrap_or("").to_owned(),
                    address_index: Value::from(target["address_index"].as_i64().unwrap_or(0)),
                    derivation_path: target["derivation_path"].as_str().map_or(Value::Null, s),
                    source: if history {
                        "derived_history"
                    } else {
                        "derived"
                    },
                    ..item.clone()
                };
                index.add_script(target["script_pubkey"].as_str().map(str::to_owned), &item);
                index.add_address(target["address"].as_str().unwrap_or(""), &item);
                if let Some(address) = target["unconfidential_address"]
                    .as_str()
                    .filter(|a| !a.is_empty())
                {
                    index.add_address(address, &item);
                }
            }
        }
    }
    seed_transaction_outpoints(conn, &mut index, profile_id)?;
    Ok((index, warnings))
}

/// `onchain.normalized_script_hex`.
fn normalized_script(value: &Value) -> Option<String> {
    if matches!(value, Value::Null) || value == &s("") {
        return None;
    }
    let text = kassiber_pytext::strip(&py_str(value)).to_lowercase();
    (text.chars().count().is_multiple_of(2) && codec::is_hex(&text)).then_some(text)
}

/// `_is_txid_token`.
fn txid_token(value: &str) -> bool {
    value.chars().count() == 64 && codec::is_hex(&value.to_lowercase())
}

/// `_seed_transaction_outpoints`: exact Core receive outpoints whose address
/// the index resolves exclusively to the row's wallet.
fn seed_transaction_outpoints(
    conn: &Connection,
    index: &mut Index,
    profile_id: &str,
) -> Result<()> {
    for row in rows(
        conn,
        "SELECT external_id, wallet_id, raw_json FROM transactions \
         WHERE profile_id = ? AND external_id IS NOT NULL AND raw_json IS NOT NULL",
        &[profile_id],
    )? {
        let txid = kassiber_pytext::strip(&or_text(&row[0])).to_lowercase();
        if !txid_token(&txid) {
            continue;
        }
        let Some(Value::List(observations)) = (match &row[2] {
            Value::Str(text) => text.as_str().and_then(|t| kassiber_pytext::loads(t).ok()),
            _ => None,
        }) else {
            continue;
        };
        let wallet_id = py_str(&row[1]);
        for observation in &observations {
            if !matches!(observation, Value::Dict(_)) {
                continue;
            }
            if kassiber_pytext::strip(&or_text(field(observation, "txid"))).to_lowercase() != txid {
                continue;
            }
            let address =
                kassiber_pytext::strip(&or_text(field(observation, "address"))).to_owned();
            let vout = field(observation, "vout");
            if address.is_empty() || matches!(vout, Value::Bool(_)) {
                continue;
            }
            let Some(position) = py_int(vout)?.filter(|n| *n >= BigInt::from(0)) else {
                continue;
            };
            let matches = index.lookup_address(&address).to_vec();
            if matches
                .iter()
                .map(|m| m.wallet_id.as_str())
                .collect::<BTreeSet<_>>()
                != BTreeSet::from([wallet_id.as_str()])
            {
                continue;
            }
            for item in matches.iter().filter(|m| m.wallet_id == wallet_id) {
                index.add_outpoint(&txid, &position, item);
            }
        }
    }
    Ok(())
}

fn pattern(cell: &'static OnceLock<regex::Regex>, source: &str) -> &'static regex::Regex {
    cell.get_or_init(|| regex::Regex::new(source).expect("static pattern"))
}

const BECH32_DATA: &str = "[02-9ac-hj-np-z]";

fn private_key(text: &str) -> bool {
    static CELL: OnceLock<regex::Regex> = OnceLock::new();
    pattern(&CELL, r"(?:[xyztuv]|[YZUV])prv[1-9A-HJ-NP-Za-km-z]{100,}").is_match(text)
}

fn bolt11(text: &str) -> bool {
    static CELL: OnceLock<regex::Regex> = OnceLock::new();
    pattern(
        &CELL,
        &format!(r"(?i)^(?:lightning:)?ln(?:bcrt|bc|tbs|tb|sb)\d*[munp]?1{BECH32_DATA}+$"),
    )
    .is_match(text)
}

fn silent_payment_address(text: &str) -> bool {
    static CELL: OnceLock<regex::Regex> = OnceLock::new();
    pattern(
        &CELL,
        &format!(r"(?i)^(?:sp|tsp|sprt)1{BECH32_DATA}{{8,}}$"),
    )
    .is_match(text)
}

/// `_unsupported_format`: recognized non-address formats, in Python's order.
fn unsupported(text: &str) -> Option<(&'static str, &'static str)> {
    static CELLS: OnceLock<Vec<(&'static str, regex::Regex, &'static str)>> = OnceLock::new();
    let formats = CELLS.get_or_init(|| {
        [
            ("lightning_offer", format!(r"(?i)^(?:lightning:)?ln[oir]1{BECH32_DATA}+$"), "BOLT12 offers and invoices are not supported yet."),
            ("lnurl", format!(r"(?i)^(?:lightning:)?lnurl1{BECH32_DATA}+$"), "LNURLs point to a server and cannot be checked on this device."),
            ("lightning_address", r"(?i)^[a-z0-9._%+-]+@[a-z0-9-]+(?:\.[a-z0-9-]+)+$".into(), "Lightning addresses point to a server and cannot be checked on this device."),
            ("outpoint", r"(?i)^[0-9a-f]{64}:\d+$".into(), "Outpoints are not supported yet; check the transaction id instead."),
            ("extended_public_key", r"^(?:[xyztuv]|[YZUV])pub[1-9A-HJ-NP-Za-km-z]{100,}$".into(), "Extended public keys are not checked here; add one as a wallet to match its addresses."),
        ]
        .into_iter()
        .map(|(kind, source, note)| (kind, regex::Regex::new(&source).expect("static pattern"), note))
        .collect()
    });
    formats
        .iter()
        .find(|(_, regex, _)| regex.is_match(text))
        .map(|(kind, _, note)| (*kind, *note))
}

fn looks_like_address_shape(text: &str) -> bool {
    static CELL: OnceLock<regex::Regex> = OnceLock::new();
    pattern(&CELL, r"^[0-9A-Za-z]{8,150}$").is_match(text)
}

fn detect_chain(address: &str) -> &'static str {
    let lower = kassiber_pytext::strip(address).to_lowercase();
    if ["lq1", "tlq1", "ex1", "tex1", "el1", "ert1"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        "liquid"
    } else if ["bc1", "tb1", "bcrt1"].iter().any(|p| lower.starts_with(p)) {
        "bitcoin"
    } else {
        ""
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Forced {
    Address,
    Txid,
    Any,
}

/// A parsed candidate (`parse_tokens`).
struct Token {
    input: String,
    normalized: String,
    kind: &'static str,
    chain: String,
    network: String,
}

/// An unparseable or unsupported entry.
struct Invalid {
    input: String,
    kind: &'static str,
    unsupported: bool,
    reason: String,
}

fn parse_tokens(
    addresses: &[String],
    txids: &[String],
    candidates: &[String],
    text: Option<&str>,
) -> (Vec<Token>, Vec<Invalid>) {
    let mut parsed = vec![];
    let mut invalid = vec![];
    let mut seen = BTreeSet::new();
    let mut add = |raw: &str, forced: Forced| {
        let text = kassiber_pytext::strip(raw);
        if text.is_empty() || text.starts_with('#') {
            return;
        }
        if private_key(text) {
            invalid.push(Invalid {
                input: "private key (not shown)".into(),
                kind: "private_key",
                unsupported: true,
                reason: PRIVATE_KEY_NOTE.into(),
            });
            return;
        }
        if forced != Forced::Txid && bolt11(text) {
            let shown = display_invoice(text);
            let Some(invoice) = decode_invoice(text) else {
                invalid.push(Invalid {
                    input: shown,
                    kind: "lightning_invoice",
                    unsupported: false,
                    reason: "not a valid Lightning invoice".into(),
                });
                return;
            };
            if seen.insert(("lightning_invoice", invoice.payment_hash.clone())) {
                parsed.push(Token {
                    input: shown,
                    normalized: invoice.payment_hash,
                    kind: "lightning_invoice",
                    chain: "lightning".into(),
                    network: invoice.network.into(),
                });
            }
            return;
        }
        if forced != Forced::Txid && silent_payment_address(text) {
            let Some((family, _, _)) = codec::decode_silent_payment_address(text) else {
                invalid.push(Invalid {
                    input: text.into(),
                    kind: "silent_payment_address",
                    unsupported: false,
                    reason: "not a valid silent payment address".into(),
                });
                return;
            };
            if seen.insert(("silent_payment_address", text.to_lowercase())) {
                parsed.push(Token {
                    input: text.into(),
                    normalized: text.to_lowercase(),
                    kind: "silent_payment_address",
                    chain: "bitcoin".into(),
                    network: family.into(),
                });
            }
            return;
        }
        if let Some((kind, note)) = (forced != Forced::Txid)
            .then(|| unsupported(text))
            .flatten()
        {
            if seen.insert((kind, text.to_lowercase())) {
                invalid.push(Invalid {
                    // BOLT12 strings carry payment details like BOLT11 ones.
                    input: if kind == "lightning_offer" {
                        display_invoice(text)
                    } else {
                        text.into()
                    },
                    kind,
                    unsupported: true,
                    reason: note.into(),
                });
            }
            return;
        }
        let kind = match forced {
            Forced::Txid => "txid",
            Forced::Address => "address",
            Forced::Any if text.chars().count() == 64 && codec::is_hex(text) => "txid",
            Forced::Any => "address",
        };
        let (normalized, key) = if kind == "txid" {
            let normalized = text.to_lowercase();
            if !txid_token(&normalized) {
                invalid.push(Invalid {
                    input: text.into(),
                    kind: "txid",
                    unsupported: false,
                    reason: "not a 64-char hex transaction id".into(),
                });
                return;
            }
            (normalized.clone(), normalized)
        } else {
            if !looks_like_address_shape(text) {
                invalid.push(Invalid {
                    input: text.into(),
                    kind: "address",
                    unsupported: false,
                    reason: "not a valid-looking address".into(),
                });
                return;
            }
            // bech32/blech32 HRPs are case-insensitive; base58 is not.
            (text.to_owned(), address_key(text))
        };
        if seen.insert((kind, key)) {
            parsed.push(Token {
                input: text.into(),
                normalized,
                kind,
                chain: if kind == "address" {
                    detect_chain(text).into()
                } else {
                    String::new()
                },
                network: String::new(),
            });
        }
    };
    for value in addresses {
        add(value, Forced::Address);
    }
    for value in txids {
        add(value, Forced::Txid);
    }
    for value in candidates {
        add(value, Forced::Any);
    }
    if let Some(text) = text.filter(|t| !t.is_empty()) {
        for line in codec::split_lines(text) {
            add(line, Forced::Any);
        }
    }
    (parsed, invalid)
}

/// `_looks_like_address`: bech32-family by HRP, base58 only when it
/// checksum-validates, so spreadsheet words and amounts are not harvested.
fn looks_like_address(value: &str) -> bool {
    let text = kassiber_pytext::strip(value);
    if text.is_empty() || !looks_like_address_shape(text) {
        return false;
    }
    let lower = text.to_lowercase();
    CASE_INSENSITIVE.iter().any(|p| lower.starts_with(p)) || address_script(text).is_some()
}

fn normalize_header(value: &str) -> String {
    value
        .replace('\u{a0}', " ")
        .split(|c: char| kassiber_pytext::is_space(c as u32))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `re.split(r"[\s,;|\t]+", text)`.
fn raw_split(text: &str) -> Vec<&str> {
    let delimiter = |c: char| kassiber_pytext::is_space(c as u32) || ",;|\t".contains(c);
    let mut parts = vec![];
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, char)) = chars.next() {
        if delimiter(char) {
            parts.push(&text[start..at]);
            let mut end = at + char.len_utf8();
            while let Some((next, char)) = chars.next_if(|(_, c)| delimiter(*c)) {
                end = next + char.len_utf8();
            }
            start = end;
        }
    }
    parts.push(&text[start..]);
    parts
}

/// `extract_candidates_from_csv`: recognized columns first, then every cell,
/// then a raw delimiter split as a fallback; strictly validated and
/// de-duplicated in first-seen order, stopping past the harvest cap.
pub fn harvest_csv(text: &str) -> Vec<String> {
    const ADDRESS_HEADERS: [&str; 13] = [
        "address",
        "addr",
        "addresses",
        "bitcoin address",
        "btc address",
        "wallet address",
        "receive address",
        "receiving address",
        "to address",
        "from address",
        "output address",
        "destination address",
        "liquid address",
    ];
    const TXID_HEADERS: [&str; 9] = [
        "txid",
        "tx id",
        "tx_id",
        "txhash",
        "tx hash",
        "transaction id",
        "transaction hash",
        "txn id",
        "txn hash",
    ];
    let stripped = text.trim_start_matches('\u{feff}');
    let rows = if text.is_empty() || kassiber_pytext::strip(stripped).is_empty() {
        vec![]
    } else {
        match crate::file_import::platform_csv::sniffed_rows(stripped) {
            (rows, None) => rows,
            (mut rows, Some(_)) => {
                rows.extend(codec::split_lines(stripped).into_iter().map(|line| {
                    raw_split(line)
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                }));
                rows
            }
        }
    };
    let mut seen = BTreeSet::new();
    let mut out: Vec<String> = vec![];
    let mut add = |value: &str| {
        let token = kassiber_pytext::strip(value);
        if !token.is_empty()
            && !seen.contains(token)
            && (txid_token(token) || looks_like_address(token))
        {
            seen.insert(token.to_owned());
            out.push(token.to_owned());
        }
        out.len() > MAX_HARVEST_CANDIDATES
    };
    if let Some(header) = rows.first() {
        let mut addresses = vec![];
        let mut txids = vec![];
        for (position, cell) in header.iter().enumerate() {
            let name = normalize_header(cell);
            if ADDRESS_HEADERS.contains(&name.as_str()) {
                addresses.push(position);
            } else if TXID_HEADERS.contains(&name.as_str()) {
                txids.push(position);
            }
        }
        for row in rows.iter().skip(1) {
            for column in addresses.iter().chain(&txids) {
                if let Some(cell) = row.get(*column) {
                    if add(cell) {
                        return out;
                    }
                }
            }
        }
        for row in &rows {
            for cell in row {
                if add(cell) {
                    return out;
                }
            }
        }
    }
    for token in raw_split(text) {
        if add(token) {
            return out;
        }
    }
    out
}

fn classify_address(token: &Token, index: &Index) -> Value {
    let mut matches = index.lookup_address(&token.input).to_vec();
    if matches.is_empty() {
        matches = index
            .lookup_script(address_script(&token.input).as_deref())
            .to_vec();
    }
    let chain = if !token.chain.is_empty() {
        token.chain.clone()
    } else {
        matches.first().map(|m| m.chain.clone()).unwrap_or_default()
    };
    if matches.is_empty() {
        return dict([
            ("input", s(token.input.as_str())),
            ("type", s("address")),
            ("chain", s(chain)),
            ("status", s("external")),
            ("classification", s("external_address")),
            ("ownership_ambiguous", Value::Bool(false)),
            ("matches", Value::List(vec![])),
            (
                "note",
                s("Not derived from or seen by any wallet in this profile."),
            ),
        ]);
    }
    let matches = sorted(&matches);
    let primary = &matches[0];
    let mut place = if primary.branch_label.is_empty() {
        "address".to_owned()
    } else {
        primary.branch_label.clone()
    };
    if primary.address_index != Value::Null {
        place = format!("{place} #{}", py_str(&primary.address_index));
    }
    let mut note = format!("Owned by '{}' ({place}).", primary.wallet_label);
    let others: BTreeSet<&str> = matches[1..]
        .iter()
        .map(|m| m.wallet_label.as_str())
        .collect();
    if !others.is_empty() {
        note += &format!(
            " Also matches: {}.",
            others.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    dict([
        ("input", s(token.input.as_str())),
        ("type", s("address")),
        (
            "chain",
            s(if chain.is_empty() {
                primary.chain.clone()
            } else {
                chain
            }),
        ),
        ("status", s("owned")),
        ("classification", s("owned_address")),
        ("ownership_ambiguous", Value::Bool(ambiguous(&matches))),
        ("canonical_wallet", s(primary.wallet_label.as_str())),
        ("canonical_wallet_id", s(primary.wallet_id.as_str())),
        (
            "matches",
            Value::List(matches.iter().map(Match::to_value).collect()),
        ),
        ("note", s(note)),
    ])
}

fn network_family(network: &str) -> &'static str {
    match network {
        "main" => "main",
        "regtest" => "regtest",
        _ => "test",
    }
}

fn classify_silent_payment(
    token: &Token,
    receivers: &[(&Wallet, Option<silent::ReceiveKeys>)],
) -> Value {
    let base = |status: &str, classification: &str| {
        vec![
            ("input", s(token.input.as_str())),
            ("type", s("silent_payment_address")),
            ("chain", s("bitcoin")),
            ("ownership_ambiguous", Value::Bool(false)),
            ("matches", Value::List(vec![])),
            ("status", s(status)),
            ("classification", s(classification)),
        ]
    };
    let finish = |mut fields: Vec<(&str, Value)>, note: String| {
        fields.push(("note", s(note)));
        dict(fields)
    };
    let Some((family, scan, spend)) = codec::decode_silent_payment_address(&token.input) else {
        return finish(
            base("invalid", "invalid"),
            "not a valid silent payment address".into(),
        );
    };
    let mut scan_only = vec![];
    let mut undecodable = false;
    for (wallet, keys) in receivers {
        let Some(keys) = keys else {
            undecodable = true;
            continue;
        };
        if keys.scan_pubkey != scan || network_family(&keys.network) != family {
            continue;
        }
        let Some((label, _)) = keys.spend_pubkeys.iter().find(|(_, key)| *key == spend) else {
            scan_only.push(wallet.label.as_str());
            continue;
        };
        let place = match label {
            None => "silent payment address".to_owned(),
            Some(0) => "change label".to_owned(),
            Some(label) => format!("label #{label}"),
        };
        let item = dict([
            ("wallet", s(wallet.label.as_str())),
            ("account", s(wallet.account.as_str())),
            ("chain", s("bitcoin")),
            ("network", s(keys.network.as_str())),
            (
                "branch",
                s(if label.is_none() {
                    "silent-payment"
                } else {
                    "label"
                }),
            ),
            (
                "address_index",
                label.map_or(Value::Null, |l| Value::from(i64::from(l))),
            ),
            ("derivation_path", Value::Null),
            ("match_source", s("silent_payment")),
        ]);
        let mut fields = base("owned", "owned_address");
        fields[4].1 = Value::List(vec![item]);
        fields.push(("canonical_wallet", s(wallet.label.as_str())));
        fields.push(("canonical_wallet_id", s(wallet.id.as_str())));
        return finish(fields, format!("Owned by '{}' ({place}).", wallet.label));
    }
    if !scan_only.is_empty() {
        let named = scan_only
            .iter()
            .map(|l| kassiber_pytext::repr_str(l))
            .collect::<Vec<_>>()
            .join(", ");
        return finish(
            base("unknown", "unknown"),
            format!(
                "Uses the scan key of {named}, but not its spend key or a label it checked (0-{}).",
                silent::LABEL_CHECK_LIMIT
            ),
        );
    }
    if undecodable {
        return finish(
            base("unknown", "unknown"),
            "A silent-payment wallet's key material could not be checked.".into(),
        );
    }
    finish(
        base("external", "external_address"),
        "Not issued by any silent-payment wallet in this profile.".into(),
    )
}

fn classify_invoice(
    conn: &Connection,
    profile_id: &str,
    token: &Token,
    wallet_ids: &BTreeSet<&str>,
) -> Result<Value> {
    let rows = rows(
        conn,
        "SELECT t.wallet_id, t.direction, w.label FROM transactions t JOIN wallets w ON w.id = t.wallet_id \
         WHERE t.profile_id = ? AND t.payment_hash = ?",
        &[profile_id, &token.normalized],
    )?;
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|row| wallet_ids.contains(py_str(&row[0]).as_str()))
        .collect();
    let mut fields = vec![
        ("input", s(token.input.as_str())),
        ("type", s("lightning_invoice")),
        ("chain", s("lightning")),
        ("ownership_ambiguous", Value::Bool(false)),
        ("matches", Value::List(vec![])),
    ];
    if rows.is_empty() {
        fields.extend([
            ("status", s("unknown")),
            ("classification", s("unknown")),
            ("note", s("Not in this profile's synced Lightning history.")),
        ]);
        return Ok(dict(fields));
    }
    let labels: BTreeSet<String> = rows.iter().map(|row| py_str(&row[2])).collect();
    let named = labels
        .iter()
        .map(|l| format!("'{l}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let directions: BTreeSet<String> = rows.iter().map(|row| or_text(&row[1])).collect();
    let (classification, note) =
        if directions.contains("inbound") && directions.contains("outbound") {
            (
                "self_transfer",
                format!("Paid between your own wallets: {named}."),
            )
        } else if directions.contains("outbound") {
            ("outbound_payment", format!("Paid by {named}."))
        } else if directions.contains("inbound") {
            ("inbound_receipt", format!("Received by {named}."))
        } else {
            ("touches_wallet", format!("Recorded by {named}."))
        };
    fields.extend([
        ("status", s("owned")),
        ("classification", s(classification)),
        ("wallets", Value::List(labels.into_iter().map(s).collect())),
        ("note", s(note)),
    ]);
    Ok(dict(fields))
}

/// Input/output scripts for one transaction (`parse_identification_legs`
/// output, or the on-chain tier's equivalent).
pub struct Legs {
    pub inputs: Vec<(Value, Value)>,
    pub outputs: Vec<(Value, Value)>,
    pub chain: String,
    pub network: String,
    pub source: String,
}

/// `parse_identification_legs` over a stored graph.
fn legs_from_graph(graph: &Json, chain: &str, network: &str) -> Result<Legs> {
    let py = |value: &Json| {
        kassiber_custody::codec::from_json(value)
            .map_err(|_| AppError::new("internal_error", "Invalid stored transaction graph"))
    };
    let entries = |key| graph[key].as_array().cloned().unwrap_or_default();
    let mut inputs = vec![];
    for entry in entries("inputs") {
        inputs.push((py(&entry["outpoint"])?, py(&entry["script"])?));
    }
    let mut outputs = vec![];
    for (position, entry) in entries("outputs").iter().enumerate() {
        let n = match entry.get("n") {
            Some(n) => py(n)?,
            None => Value::from(position as i64),
        };
        outputs.push((n, py(&entry["script"])?));
    }
    let text = |key: &str| {
        graph[key]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    Ok(Legs {
        inputs,
        outputs,
        chain: text("chain").unwrap_or_else(|| {
            if chain.is_empty() {
                "bitcoin".into()
            } else {
                chain.into()
            }
        }),
        network: text("network").unwrap_or_else(|| network.into()),
        source: "local_tx".into(),
    })
}

/// `_wallet_chain_network`.
fn wallet_chain_network(config: &Value, kind: &Value) -> (String, String) {
    let config = match config {
        Value::Str(text) if truthy(config) => {
            text.as_str().and_then(|t| kassiber_pytext::loads(t).ok())
        }
        _ => None,
    }
    .filter(|c| matches!(c, Value::Dict(_)))
    .unwrap_or_else(|| dict::<&str>([]));
    let mut chain = {
        let value = field(&config, "chain");
        if truthy(value) {
            py_str(value)
        } else {
            "bitcoin".into()
        }
    }
    .to_lowercase();
    let mut network = {
        let value = field(&config, "network");
        if truthy(value) {
            py_str(value)
        } else {
            "main".into()
        }
    }
    .to_lowercase();
    if or_text(kind).to_lowercase().contains("liquid") {
        chain = "liquid".into();
        if ["", "main", "mainnet"].contains(&network.as_str()) {
            network = "liquidv1".into();
        }
    } else if ["", "btc"].contains(&chain.as_str()) {
        chain = "bitcoin".into();
    }
    descriptors::normalize_chain(&chain)
        .and_then(|c| descriptors::normalize_network(&c, &network).map(|n| (c, n)))
        .unwrap_or((chain, network))
}

/// `load_local_tx_legs`.
fn local_legs(conn: &Connection, profile_id: &str, txid: &str) -> Result<Option<Legs>> {
    for row in rows(
        conn,
        "SELECT t.raw_json, w.kind AS wallet_kind, w.config_json AS wallet_config_json \
         FROM transactions t LEFT JOIN wallets w ON w.id = t.wallet_id WHERE t.profile_id = ? AND t.external_id = ?",
        &[profile_id, txid],
    )? {
        let (chain, network) = wallet_chain_network(&row[2], &row[1]);
        let raw = match &row[0] {
            Value::Null => continue,
            other => kassiber_custody::codec::to_json(other),
        };
        let graph = kassiber_custody::journal_support::parsed(&serde_json::json!({ "raw_json": raw }))
            .map_err(|_| AppError::new("internal_error", "Invalid stored transaction graph"))?;
        if let Some(graph) = graph {
            return Ok(Some(legs_from_graph(&graph, &chain, &network)?));
        }
    }
    Ok(None)
}

/// `classify_txid`: per-leg ownership from local or on-chain legs, or the
/// local inventory's wallets when no legs are known.
pub fn classify_txid(
    token_input: &str,
    token_chain: &str,
    token_network: &str,
    index: &Index,
    legs: Option<&Legs>,
) -> Value {
    let txid = token_input;
    let mut local = index.lookup_txid_wallets(txid, None);
    let Some(legs) = legs else {
        let wallets: Vec<String> = local
            .iter()
            .map(|(_, label)| label.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let owned = !wallets.is_empty();
        let note = if owned {
            format!(
                "Recorded against {}; per-leg breakdown needs on-chain verification.",
                wallets
                    .iter()
                    .map(|w| format!("'{w}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            "Not in this profile's synced/imported history; on-chain verification is needed for a verdict.".into()
        };
        return dict([
            ("input", s(txid)),
            ("type", s("txid")),
            ("chain", s("")),
            ("status", s(if owned { "owned" } else { "unknown" })),
            (
                "classification",
                s(if owned { "touches_wallet" } else { "unknown" }),
            ),
            ("ownership_ambiguous", Value::Bool(false)),
            ("ambiguous_legs", Value::from(0)),
            ("wallets", Value::List(wallets.into_iter().map(s).collect())),
            ("owned_inputs", Value::Null),
            ("owned_outputs", Value::Null),
            ("external_outputs", Value::Null),
            ("legs", Value::List(vec![])),
            ("match_source", s(if owned { "inventory" } else { "none" })),
            ("note", s(note)),
        ]);
    };
    let chain = if legs.chain.is_empty() {
        token_chain.to_owned()
    } else {
        legs.chain.clone()
    };
    let network = if legs.network.is_empty() {
        token_network.to_owned()
    } else {
        legs.network.clone()
    };
    let scope = match_scope(&chain, &network);
    if let Some(scope) = &scope {
        local = index.lookup_txid_wallets(txid, Some(scope));
    }
    let source = if legs.source.is_empty() {
        "chain".to_owned()
    } else {
        legs.source.clone()
    };
    let mut involved = BTreeSet::new();
    let (mut owned_in, mut unresolved, mut ambiguous_legs) = (0i64, 0i64, 0i64);
    let mut rendered = vec![];
    for (outpoint, script) in &legs.inputs {
        let outpoint_text = or_text(outpoint);
        let by_outpoint = index.lookup_outpoint(&outpoint_text, scope.as_ref());
        let mut matches = in_scope(&by_outpoint, scope.as_ref());
        if matches.is_empty() {
            let script_text = matches!(script, Value::Str(_)).then(|| py_str(script));
            matches = in_scope(index.lookup_script(script_text.as_deref()), scope.as_ref());
            if matches.is_empty() && *script == Value::Null && source != "local_tx" {
                unresolved += 1;
            }
        }
        let leg_ambiguous = !matches.is_empty() && ambiguous(&matches);
        owned_in += i64::from(!matches.is_empty());
        ambiguous_legs += i64::from(leg_ambiguous);
        involved.extend(labels(&matches));
        let mut row = vec![
            ("side", s("input")),
            ("outpoint", outpoint.clone()),
            ("owned", Value::Bool(!matches.is_empty())),
            (
                "wallet",
                s(matches.first().map_or("", |m| m.wallet_label.as_str())),
            ),
            ("ownership_ambiguous", Value::Bool(leg_ambiguous)),
        ];
        if let Some(primary) = matches.first() {
            row.push((
                "wallets",
                Value::List(labels(&matches).into_iter().map(s).collect()),
            ));
            row.push(("canonical_wallet_id", s(primary.wallet_id.as_str())));
        }
        rendered.push(dict(row));
    }
    let mut owned_out = 0i64;
    let mut outputs = 0i64;
    for (n, script) in &legs.outputs {
        // An empty script is Liquid's fee output; OP_RETURN data is unspendable.
        if !truthy(script)
            || kassiber_pytext::strip(&py_str(script))
                .to_lowercase()
                .starts_with("6a")
        {
            continue;
        }
        outputs += 1;
        let script_text = matches!(script, Value::Str(_)).then(|| py_str(script));
        let matches = in_scope(index.lookup_script(script_text.as_deref()), scope.as_ref());
        let leg_ambiguous = !matches.is_empty() && ambiguous(&matches);
        owned_out += i64::from(!matches.is_empty());
        ambiguous_legs += i64::from(leg_ambiguous);
        involved.extend(labels(&matches));
        let mut row = vec![
            ("side", s("output")),
            ("n", n.clone()),
            ("owned", Value::Bool(!matches.is_empty())),
            (
                "wallet",
                s(matches.first().map_or("", |m| m.wallet_label.as_str())),
            ),
            (
                "branch",
                s(matches.first().map_or("", |m| m.branch_label.as_str())),
            ),
            ("ownership_ambiguous", Value::Bool(leg_ambiguous)),
        ];
        if let Some(primary) = matches.first() {
            row.push((
                "wallets",
                Value::List(labels(&matches).into_iter().map(s).collect()),
            ));
            row.push(("canonical_wallet_id", s(primary.wallet_id.as_str())));
        }
        rendered.push(dict(row));
    }
    involved.extend(local.iter().map(|(_, label)| label.clone()));
    let external_out = outputs - owned_out;
    let (classification, status, note) = classify_legs(
        owned_in,
        owned_out,
        external_out,
        unresolved,
        !local.is_empty(),
    );
    dict([
        ("input", s(txid)),
        ("type", s("txid")),
        ("chain", s(chain)),
        ("network", s(network)),
        ("status", s(status)),
        ("classification", s(classification)),
        ("ownership_ambiguous", Value::Bool(ambiguous_legs > 0)),
        ("ambiguous_legs", Value::from(ambiguous_legs)),
        (
            "wallets",
            Value::List(involved.into_iter().map(s).collect()),
        ),
        ("owned_inputs", Value::from(owned_in)),
        ("owned_outputs", Value::from(owned_out)),
        ("external_outputs", Value::from(external_out)),
        ("legs", Value::List(rendered)),
        ("match_source", s(source)),
        ("note", s(note)),
    ])
}

fn classify_legs(
    owned_in: i64,
    owned_out: i64,
    external_out: i64,
    unresolved: i64,
    touches: bool,
) -> (&'static str, &'static str, &'static str) {
    if owned_in == 0 && owned_out == 0 {
        if touches {
            return ("touches_wallet", "owned", "Recorded locally against this profile, but verified legs did not match known wallet scripts.");
        }
        return (
            "external",
            "external",
            "No inputs or outputs belong to this profile.",
        );
    }
    if owned_out == 0 && external_out == 0 {
        return (
            "undetermined",
            "owned",
            "Inputs are owned but no outputs were resolved; classification unavailable.",
        );
    }
    if owned_in == 0 && owned_out > 0 {
        if unresolved > 0 {
            return (
                "undetermined",
                "owned",
                "Outputs include an owned address, but one or more inputs could not be resolved.",
            );
        }
        return (
            "inbound_receipt",
            "owned",
            "Inbound: funds received to an owned address from an external sender.",
        );
    }
    if owned_in > 0 && external_out == 0 {
        return (
            "self_transfer",
            "owned",
            "Self-transfer/consolidation: all outputs return to owned addresses.",
        );
    }
    if owned_in > 0 && owned_out > 0 {
        return ("outbound_payment", "owned", "Outbound payment: spent from owned inputs to an external recipient (owned outputs are change).");
    }
    (
        "outbound_payment",
        "owned",
        "Outbound payment: spent from owned inputs entirely to external recipients.",
    )
}

/// A txid local data could not settle, left for the on-chain tier.
pub struct Pending {
    pub position: usize,
    pub txid: String,
    pub chain: String,
    pub network: String,
}

/// `IdentifyPlan`: everything local data settled, and what is pending.
pub struct Plan {
    pub results: Vec<Option<Value>>,
    pub pending: Vec<Pending>,
    pub index: Index,
    pub wallet_count: usize,
    pub scan_to_index: i64,
    pub warnings: Vec<String>,
    pub verify: bool,
}

/// `prepare_identify`. With `verify`, txids missing from local history are
/// left pending; without it they resolve to "unknown" here.
pub fn prepare(conn: &Connection, profile_id: &str, inputs: &Inputs, verify: bool) -> Result<Plan> {
    let scan_to_index = inputs.scan_to_index.clamp(0, MAX_SCAN_TO_INDEX);
    let mut warnings = vec![];
    let mut candidates = inputs.candidates.clone();
    if let Some(csv) = inputs.csv_text.as_deref().filter(|t| !t.is_empty()) {
        let mut harvested = harvest_csv(csv);
        if harvested.len() > MAX_HARVEST_CANDIDATES {
            warnings.push(format!(
                "CSV yielded {} candidates; only the first {MAX_HARVEST_CANDIDATES} were checked.",
                harvested.len()
            ));
            harvested.truncate(MAX_HARVEST_CANDIDATES);
        }
        candidates.extend(harvested);
    }
    let (parsed, invalid) = parse_tokens(
        &inputs.addresses,
        &inputs.txids,
        &candidates,
        inputs.text.as_deref(),
    );
    let wallets = load_wallets(conn, profile_id)?;
    let (index, built) = build_index(conn, profile_id, &wallets, scan_to_index)?;
    warnings.extend(built);
    let wallet_ids: BTreeSet<&str> = wallets.iter().map(|w| w.id.as_str()).collect();
    let mut receivers = None;
    let mut results = vec![];
    let mut pending = vec![];
    for token in &parsed {
        let result = match token.kind {
            "address" => classify_address(token, &index),
            "lightning_invoice" => classify_invoice(conn, profile_id, token, &wallet_ids)?,
            "silent_payment_address" => {
                if receivers.is_none() {
                    let mut found = vec![];
                    for wallet in &wallets {
                        let config = wallet_config(&wallet.config)?;
                        let material = field(&config, "sp_descriptor");
                        if *material == Value::Null
                            || kassiber_pytext::strip(&py_str(material)).is_empty()
                        {
                            continue;
                        }
                        let config = kassiber_custody::codec::to_json(&config);
                        found.push((wallet, silent::receive_keys(&config)?));
                    }
                    receivers = Some(found);
                }
                classify_silent_payment(token, receivers.as_deref().unwrap_or_default())
            }
            _ => match local_legs(conn, profile_id, &token.normalized)? {
                None if verify => {
                    pending.push(Pending {
                        position: results.len(),
                        txid: token.normalized.clone(),
                        chain: token.chain.clone(),
                        network: token.network.clone(),
                    });
                    results.push(None);
                    continue;
                }
                legs => classify_txid(
                    &token.normalized,
                    &token.chain,
                    &token.network,
                    &index,
                    legs.as_ref(),
                ),
            },
        };
        results.push(Some(result));
    }
    for entry in invalid {
        let status = if entry.unsupported {
            "unsupported"
        } else {
            "invalid"
        };
        results.push(Some(dict([
            ("input", s(entry.input)),
            ("type", s(entry.kind)),
            ("chain", s("")),
            ("status", s(status)),
            ("classification", s(status)),
            ("note", s(entry.reason)),
        ])));
    }
    Ok(Plan {
        results,
        pending,
        index,
        wallet_count: wallets.len(),
        scan_to_index,
        warnings,
        verify,
    })
}

/// `summarize`.
pub fn summarize(results: &[Value], wallets: usize, scan_to_index: i64, verified: bool) -> Value {
    let count = |status: &str| {
        Value::from(
            results
                .iter()
                .filter(|r| kassiber_envelope::get(r, "status") == Some(&s(status)))
                .count() as i64,
        )
    };
    dict([
        ("total", Value::from(results.len() as i64)),
        ("owned", count("owned")),
        ("external", count("external")),
        ("unknown", count("unknown")),
        ("invalid", count("invalid")),
        ("unsupported", count("unsupported")),
        ("wallets_scanned", Value::from(wallets as i64)),
        ("scan_to_index", Value::from(scan_to_index)),
        ("verified_on_chain", Value::Bool(verified)),
    ])
}

/// Why one on-chain lookup produced no legs.
pub enum LookupError {
    /// A broken configuration, not a per-txid miss: the run fails.
    Refused(AppError),
    /// A per-txid miss, reported as a warning; that txid stays unknown.
    Missed(String),
}

/// One lookup of a pending txid on its backend: `(txid, chain hint)`.
pub type Lookup<'a> = dyn FnMut(&str, &str) -> std::result::Result<Option<Legs>, LookupError> + 'a;

impl Plan {
    /// `finish_identify` without lookups: the report local data supports.
    pub fn report(self) -> Value {
        self.finish(None, &mut |_, _| Ok(()), &|| false)
            .expect("a run without lookups cannot fail")
    }

    /// `finish_identify`: look the pending txids up one at a time. `cancelled`
    /// is checked before each lookup; once it holds, the rest stay unknown.
    pub fn finish(
        mut self,
        mut lookup: Option<&mut Lookup<'_>>,
        progress: &mut dyn FnMut(usize, usize) -> Result<()>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Value> {
        let total = self.pending.len();
        for checked in 0..total {
            if cancelled() {
                self.warnings.push(format!(
                    "Stopped after {checked} of {total} on-chain lookups; the rest stay unknown."
                ));
                for later in &self.pending[checked..] {
                    self.results[later.position] = Some(classify_txid(
                        &later.txid,
                        &later.chain,
                        &later.network,
                        &self.index,
                        None,
                    ));
                }
                break;
            }
            let token = &self.pending[checked];
            let mut legs = None;
            if let Some(lookup) = lookup.as_mut() {
                match lookup(&token.txid, &token.chain) {
                    Ok(found) => legs = found,
                    Err(LookupError::Refused(error)) => return Err(error),
                    Err(LookupError::Missed(reason)) => {
                        let short: String = token.txid.chars().take(12).collect();
                        self.warnings
                            .push(format!("On-chain verify for {short}…: {reason}"));
                    }
                }
            }
            self.results[token.position] = Some(classify_txid(
                &token.txid,
                &token.chain,
                &token.network,
                &self.index,
                legs.as_ref(),
            ));
            progress(checked + 1, total)?;
        }
        let results: Vec<Value> = self.results.into_iter().flatten().collect();
        let summary = summarize(&results, self.wallet_count, self.scan_to_index, self.verify);
        Ok(dict([
            ("results", Value::List(results)),
            ("summary", summary),
            (
                "warnings",
                Value::List(self.warnings.into_iter().map(s).collect()),
            ),
        ]))
    }
}

/// `_empty_identify_payload`: no active profile.
pub fn empty() -> Value {
    let mut payload = Plan {
        results: vec![],
        pending: vec![],
        index: Index::default(),
        wallet_count: 0,
        scan_to_index: 0,
        warnings: vec![],
        verify: false,
    }
    .report();
    kassiber_envelope::set(
        &mut payload,
        "context",
        dict([("workspace", Value::Null), ("profile", Value::Null)]),
    );
    payload
}

/// `ui.wallets.identify`: the local, cache-only reconciliation.
pub fn identify(conn: &Connection, profile_id: &str, inputs: &Inputs) -> Result<Value> {
    Ok(prepare(conn, profile_id, inputs, false)?.report())
}

#[cfg(test)]
mod tests;
