//! Bitcoin-family transfer scope and Bitcoin evidence fee/receipt normalization.
use crate::codec::{self, int, text, truth};
use crate::evidence::{self, EventKey};
use crate::python_text::PythonText;
use crate::snapshot::Snapshot;
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::str::FromStr;

pub(crate) mod ownership;
pub(crate) mod stored;

fn graph_integer(value: &Value) -> BigInt {
    BigInt::from_str(&value.to_string()).expect("converted graph integer")
}
fn integer_value(value: &BigInt) -> Value {
    serde_json::from_str(&value.to_string()).expect("computed integer")
}
pub(crate) fn sats(entry: &Value) -> Option<i64> {
    let explicit = !entry["value_sats"].is_null();
    let value = if explicit {
        &entry["value_sats"]
    } else {
        &entry["value"]
    };
    if value.is_null() || value == "" || value.is_boolean() {
        return None;
    }
    let spelling = codec::pystr(value);
    if explicit
        || value
            .as_number()
            .is_some_and(|n| !n.as_str().contains(['.', 'e', 'E']))
        || value.is_string() && !spelling.contains('.')
    {
        return spelling.py_strip().parse::<i64>().ok().or_else(|| {
            if explicit {
                value.as_f64().filter(|v| v.is_finite()).map(|v| v as i64)
            } else {
                None
            }
        });
    }
    let spelling = if value.is_number() {
        codec::pystr(value)
    } else {
        spelling
    };
    Decimal::from_str(&spelling)
        .ok()?
        .mul(&Decimal::from(100_000_000), &Context::RP2)
        .ok()?
        .to_fixed_places(0, Rounding::HalfEven)
        .ok()?
        .parse()
        .ok()
}
pub(crate) fn strict_txid(v: &Value) -> Option<String> {
    let s = text(v).py_strip().py_lower();
    if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(s)
    } else {
        None
    }
}
#[cfg(test)]
fn merge(items: &[Value]) -> Option<Value> {
    let items = items
        .iter()
        .map(codec::from_json)
        .collect::<Result<Vec<_>, _>>()
        .expect("JSON graphs");
    stored::merge(&items).as_ref().map(stored::project)
}
pub(crate) fn parse(raw: &Value) -> Result<Option<Value>, String> {
    Ok(stored::parse(raw)?.as_ref().map(stored::project))
}
// Transfer identity predicates stringify nonfinite Python scalars. Preserve
// their presence instead of turning them into null during outer JSON conversion.
fn decoded(value: &kassiber_pyjson::Value) -> Value {
    use kassiber_pyjson::Value as Py;
    match value {
        Py::Float(n) if !n.is_finite() => json!(kassiber_pyjson::float_repr(*n)),
        Py::List(values) => json!(values.iter().map(decoded).collect::<Vec<_>>()),
        Py::Dict(values) => Value::Object(
            values
                .iter()
                .filter_map(|(k, v)| {
                    if let kassiber_pyjson::Key::Str(k) = k {
                        Some((k.as_str()?.to_owned(), decoded(v)))
                    } else {
                        None
                    }
                })
                .collect(),
        ),
        _ => codec::to_json(value),
    }
}

/// A raw JSON text of the operation, loaded once: its `codec::loads` and,
/// built on first request, its transfer decoding.
#[derive(Debug)]
pub(crate) struct RawText {
    loaded: Result<kassiber_pyjson::Value, ()>,
    /// The text, kept only when it does not load; it decodes as itself.
    unparsed: Option<Box<str>>,
    decoded: std::sync::OnceLock<Value>,
}

static EMPTY_OBJECT: std::sync::LazyLock<Value> =
    std::sync::LazyLock::new(|| Value::Object(serde_json::Map::new()));
static NULL: Value = Value::Null;

impl RawText {
    fn new(text: &str) -> Self {
        let loaded = codec::loads(text).map_err(|_| ());
        Self {
            unparsed: loaded.is_err().then(|| text.into()),
            loaded,
            decoded: std::sync::OnceLock::new(),
        }
    }

    pub(crate) fn loaded(&self) -> &Result<kassiber_pyjson::Value, ()> {
        &self.loaded
    }

    fn decoded(&self) -> &Value {
        self.decoded.get_or_init(|| match &self.loaded {
            Ok(value) => decoded(value),
            Err(()) => Value::String(self.unparsed.as_deref().unwrap_or_default().into()),
        })
    }

    fn is_object(&self) -> bool {
        matches!(self.loaded, Ok(kassiber_pyjson::Value::Dict(_)))
    }

    /// The decoded field `key`, decoding only that field: the last entry
    /// with that key, as the decoded mapping keeps it.
    fn field(&self, key: &str) -> std::borrow::Cow<'_, Value> {
        use std::borrow::Cow;
        if let Some(decoded) = self.decoded.get() {
            return Cow::Borrowed(&decoded[key]);
        }
        let Ok(kassiber_pyjson::Value::Dict(items)) = &self.loaded else {
            return Cow::Borrowed(&NULL);
        };
        items
            .iter()
            .rev()
            .find_map(|(k, v)| match k {
                // Equal bytes are valid UTF-8, as `key` is.
                kassiber_pyjson::Key::Str(k) if k.is(key) => Some(v),
                _ => None,
            })
            .map_or(Cow::Borrowed(&NULL), |v| Cow::Owned(decoded(v)))
    }
}

type RawEntry = (std::sync::Arc<str>, std::sync::Arc<RawText>);
thread_local! {
    static RAW_TEXTS: std::cell::RefCell<crate::fxhash::HashMap<std::sync::Arc<str>, std::sync::Arc<RawText>>> =
        std::cell::RefCell::new(Default::default());
    /// The entry last found at a text's address. A hit still compares the
    /// text, so a reused address cannot return another text's entry.
    static RAW_ADDRESSES: std::cell::RefCell<crate::fxhash::HashMap<usize, RawEntry>> =
        std::cell::RefCell::new(Default::default());
}

/// The operation's loaded raw text `text`, shared by every reader.
pub(crate) fn raw_text(text: &str) -> std::sync::Arc<RawText> {
    let address = text.as_ptr() as usize;
    if let Some(hit) = RAW_ADDRESSES.with(|m| {
        m.borrow()
            .get(&address)
            .filter(|(known, _)| **known == *text)
            .map(|(_, entry)| entry.clone())
    }) {
        return hit;
    }
    // Only a text's first sighting is remembered by address: an equal text
    // elsewhere is found by its content without growing the address map.
    let (key, entry) = RAW_TEXTS.with(|m| {
        if let Some(entry) = m.borrow().get(text) {
            return (None, entry.clone());
        }
        let key: std::sync::Arc<str> = text.into();
        let entry = std::sync::Arc::new(RawText::new(text));
        m.borrow_mut().insert(key.clone(), entry.clone());
        (Some(key), entry)
    });
    if let Some(key) = key {
        RAW_ADDRESSES.with(|m| m.borrow_mut().insert(address, (key, entry.clone())));
    }
    entry
}

pub(crate) enum TransferValue<'a> {
    Borrowed(&'a Value),
    Raw(std::sync::Arc<RawText>),
    Empty,
}

impl Deref for TransferValue<'_> {
    type Target = Value;

    fn deref(&self) -> &Value {
        match self {
            Self::Borrowed(value) => value,
            Self::Raw(raw) => raw.decoded(),
            Self::Empty => &EMPTY_OBJECT,
        }
    }
}

impl TransferValue<'_> {
    /// `self[key]`, without decoding the rest of a raw text.
    pub(crate) fn field(&self, key: &str) -> std::borrow::Cow<'_, Value> {
        use std::borrow::Cow;
        match self {
            Self::Borrowed(value) => Cow::Borrowed(&value[key]),
            Self::Raw(raw) => raw.field(key),
            Self::Empty => Cow::Borrowed(&NULL),
        }
    }
}

fn transfer_value(value: &Value) -> TransferValue<'_> {
    match value.as_str() {
        Some(text) => TransferValue::Raw(raw_text(text)),
        None => TransferValue::Borrowed(value),
    }
}
pub fn transfer_object(value: &Value) -> Value {
    (*transfer_object_ref(value)).clone()
}

/// `transfer_object_ref` of a string whose shared loaded text is `raw`.
pub(crate) fn raw_object<'a>(raw: &std::sync::Arc<RawText>) -> TransferValue<'a> {
    if raw.is_object() {
        TransferValue::Raw(raw.clone())
    } else {
        TransferValue::Empty
    }
}

pub(crate) fn transfer_object_ref(value: &Value) -> TransferValue<'_> {
    // The empty object's text needs no loading.
    if value.as_str() == Some("{}") {
        return TransferValue::Empty;
    }
    let value = transfer_value(value);
    let object = match &value {
        TransferValue::Raw(raw) => raw.is_object(),
        value => value.is_object(),
    };
    if object {
        value
    } else {
        TransferValue::Empty
    }
}

pub(crate) type Scope = Option<(String, String, String, String)>;
/// The row fields a scope reads.
const SCOPE_FIELDS: [&str; 8] = [
    "id",
    "raw_json",
    "config_json",
    "wallet_config_json",
    "external_id",
    "external_id_kind",
    "wallet_kind",
    "asset",
];
type ScopeFields = [Value; 8];
thread_local! {
    static SCOPES: std::cell::RefCell<crate::fxhash::HashMap<String, Scope>> =
        std::cell::RefCell::new(Default::default());
    /// A row address hint: a hit still compares every scope field.
    static SCOPE_ROWS: std::cell::RefCell<crate::fxhash::HashMap<usize, (Box<ScopeFields>, Scope)>> =
        std::cell::RefCell::new(Default::default());
}

/// Scopes of the operation's snapshot rows by position. The rows cannot
/// change while an operation holds its snapshot, so a row found by address
/// needs no field comparison.
struct RowScopes {
    /// The JSON rows' address once they are built; zero before.
    start: std::cell::Cell<usize>,
    records: usize,
    len: usize,
    scopes: Vec<std::cell::OnceCell<Scope>>,
}
thread_local! {
    static ROW_SCOPES: std::cell::RefCell<Option<std::rc::Rc<RowScopes>>> =
        const { std::cell::RefCell::new(None) };
}

/// Registers the snapshot rows for one operation; dropping it ends that.
pub(crate) struct RowScopesGuard(Option<std::rc::Rc<RowScopes>>);
/// The snapshot's JSON rows, if already built, hold the records' fields in
/// the same order; rows built later are noted by `note_rows`.
pub(crate) fn register_rows(
    rows: Option<&[Value]>,
    records: &[crate::snapshot::Record],
) -> RowScopesGuard {
    let table = std::rc::Rc::new(RowScopes {
        start: std::cell::Cell::new(0),
        records: records.as_ptr() as usize,
        len: records.len(),
        scopes: (0..records.len())
            .map(|_| std::cell::OnceCell::new())
            .collect(),
    });
    // Each transaction loads about one raw text; size the shared tables once.
    RAW_TEXTS.with(|m| m.borrow_mut().reserve(records.len()));
    RAW_ADDRESSES.with(|m| m.borrow_mut().reserve(records.len()));
    let guard = RowScopesGuard(ROW_SCOPES.with(|r| r.replace(Some(table))));
    if let Some(rows) = rows {
        note_rows(records, rows);
    }
    guard
}
/// Notes the JSON rows built from the registered records.
pub(crate) fn note_rows(records: &[crate::snapshot::Record], rows: &[Value]) {
    ROW_SCOPES.with(|r| {
        if let Some(table) = r.borrow().as_ref() {
            if table.records == records.as_ptr() as usize
                && table.len == records.len()
                && rows.len() == records.len()
            {
                table.start.set(rows.as_ptr() as usize);
            }
        }
    });
}
impl Drop for RowScopesGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        ROW_SCOPES.with(|r| *r.borrow_mut() = previous);
    }
}
fn registered(row: &Value) -> Option<(std::rc::Rc<RowScopes>, usize)> {
    let table = ROW_SCOPES.with(|r| r.borrow().clone())?;
    let start = table.start.get();
    if start == 0 {
        return None;
    }
    let offset = (row as *const Value as usize).checked_sub(start)?;
    let at = offset / std::mem::size_of::<Value>();
    (offset % std::mem::size_of::<Value>() == 0 && at < table.len).then_some((table, at))
}
fn registered_record(row: &crate::snapshot::Record) -> Option<(std::rc::Rc<RowScopes>, usize)> {
    let table = ROW_SCOPES.with(|r| r.borrow().clone())?;
    let size = std::mem::size_of::<crate::snapshot::Record>();
    let offset = (row as *const crate::snapshot::Record as usize).checked_sub(table.records)?;
    let at = offset / size;
    (offset % size == 0 && at < table.len).then_some((table, at))
}

/// `onchain_scope` of a snapshot record, which holds its row's fields; a
/// registered record shares its row's scope slot.
pub(crate) fn record_scope(row: &crate::snapshot::Record) -> Scope {
    match registered_record(row) {
        Some((table, at)) => table.scopes[at].get_or_init(|| physical_scope(row)).clone(),
        None => physical_scope(row),
    }
}

/// Releases memoized scopes at the end of an operation.
pub(crate) fn clear_scopes() {
    RAW_ADDRESSES.with(|m| m.borrow_mut().clear());
    RAW_TEXTS.with(|m| m.borrow_mut().clear());
    SCOPES.with(|m| m.borrow_mut().clear());
    SCOPE_ROWS.with(|m| m.borrow_mut().clear());
}

/// Physical Bitcoin-family grouping; contradictory graph/configuration facts refuse it.
/// The scope reads only these row fields, so it is memoized by their exact JSON.
pub fn onchain_scope(row: &Value) -> Scope {
    if let Some((table, at)) = registered(row) {
        return table.scopes[at].get_or_init(|| physical_scope(row)).clone();
    }
    // The same row is probed many times; its address finds its fields without
    // encoding them, and a hit requires every field to be equal.
    let address = row as *const Value as usize;
    let hit = SCOPE_ROWS.with(|m| {
        m.borrow()
            .get(&address)
            .filter(|(fields, _)| fields.iter().zip(SCOPE_FIELDS).all(|(v, k)| *v == row[k]))
            .map(|(_, scope)| scope.clone())
    });
    if let Some(hit) = hit {
        return hit;
    }
    let key = scope_key(row);
    let scope = match SCOPES.with(|m| m.borrow().get(&key).cloned()) {
        Some(hit) => hit,
        None => {
            let scope = physical_scope(row);
            SCOPES.with(|m| m.borrow_mut().insert(key, scope.clone()));
            scope
        }
    };
    let fields = Box::new(SCOPE_FIELDS.map(|k| row[k].clone()));
    SCOPE_ROWS.with(|m| m.borrow_mut().insert(address, (fields, scope.clone())));
    scope
}

fn scope_key(row: &Value) -> String {
    serde_json::to_string(&[
        &row["id"],
        &row["raw_json"],
        &row["config_json"],
        &row["wallet_config_json"],
        &row["external_id"],
        &row["external_id_kind"],
        &row["wallet_kind"],
        &row["asset"],
    ])
    .expect("JSON values")
}

fn physical_scope<R: crate::fields::Fields + ?Sized>(row: &R) -> Scope {
    if [
        "custody-tax:",
        "cross-split:",
        "direct-payout:",
        "multi-consol:",
        "owned-derive:",
        "recorded-fanout:",
    ]
    .iter()
    .any(|p| text(row.at("id")).starts_with(p))
    {
        return None;
    }
    // A raw object without nested graphs is its only payload. With an
    // invalid txid, or none and no txid external id, it has no scope; this
    // reads those fields without decoding the rest of the text.
    if let Some(raw_json) = row.at("raw_json").as_str() {
        let raw = raw_text(raw_json);
        if raw.is_object()
            && !raw.field("tx").is_object()
            && !raw.field("ownership_graph").is_object()
        {
            let txid = raw.field("txid");
            if !text(&txid).py_strip().is_empty() {
                strict_txid(&txid)?;
            } else if strict_txid(row.at("external_id")).is_none() {
                return None;
            }
        }
    }
    let raw = transfer_object_ref(row.at("raw_json"));
    let config = transfer_object_ref(if truth(row.at("config_json")) {
        row.at("config_json")
    } else {
        row.at("wallet_config_json")
    });
    let decoded = transfer_value(row.at("raw_json"));
    let mut maps = vec![&*raw];
    maps.extend(
        decoded
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v.is_object()),
    );
    for key in ["tx", "ownership_graph"] {
        if raw[key].is_object() {
            maps.push(&raw[key]);
        }
    }
    let mut txids = BTreeSet::new();
    let mut chains = BTreeSet::new();
    let mut networks = BTreeSet::new();
    for payload in &maps {
        if !text(&payload["txid"]).py_strip().is_empty() {
            txids.insert(strict_txid(&payload["txid"])?);
        }
    }
    if txids.len() > 1 {
        return None;
    }
    let external = strict_txid(row.at("external_id"));
    let graph = txids.into_iter().next();
    if graph.is_some() && external.is_some() && graph != external {
        return None;
    }
    if graph.is_none() && external.is_some() {
        let kind = if truth(row.at("external_id_kind")) {
            text(row.at("external_id_kind"))
        } else {
            text(&raw["external_id_kind"])
        };
        let kind = kind.py_strip().py_lower();
        let typed = if !kind.is_empty() {
            ["txid", "transaction_hash", "onchain_txid"].contains(&kind.as_str())
        } else {
            maps.iter()
                .filter_map(|v| v.as_object())
                .flat_map(|m| m.iter())
                .any(|(k, v)| {
                    let name: String = k
                        .py_lower()
                        .chars()
                        .filter(|c| crate::python_text::isalnum(*c))
                        .collect();
                    [
                        "txid",
                        "txhash",
                        "transactionid",
                        "transactionhash",
                        "onchaintxid",
                        "sendtxid",
                        "lockuptxid",
                        "receivetxid",
                        "claimtxid",
                        "refundtxid",
                    ]
                    .contains(&name.as_str())
                        && strict_txid(v) == external
                })
        };
        if !typed {
            return None;
        }
    }
    let txid = graph.or(external)?;
    let mut kind = text(row.at("wallet_kind"))
        .py_strip()
        .py_lower()
        .replace('_', "-");
    if ["core-ln", "core-lightning"].contains(&kind.as_str()) {
        kind = "coreln".into();
    }
    if evidence::lightning(&kind)
        && !maps
            .iter()
            .any(|v| v["vin"].is_array() && v["vout"].is_array())
    {
        return None;
    }
    for payload in maps.iter().copied().chain(std::iter::once(&*config)) {
        let c = text(&payload["chain"]);
        if !c.py_strip().is_empty() {
            let c = evidence::chain(&c.py_strip().py_lower()).ok()?;
            if c == "lightning" {
                return None;
            }
            chains.insert(c);
        }
    }
    if chains.len() > 1 {
        return None;
    }
    let asset = text(row.at("asset"))
        .py_strip()
        .py_upper()
        .replace("L-BTC", "LBTC");
    let chain = chains.into_iter().next().or_else(|| match asset.as_str() {
        "BTC" => Some("bitcoin".into()),
        "LBTC" => Some("liquid".into()),
        _ => None,
    })?;
    if (asset == "BTC" && chain != "bitcoin") || (asset == "LBTC" && chain != "liquid") {
        return None;
    }
    for payload in maps.iter().copied().chain(std::iter::once(&*config)) {
        if !text(&payload["network"]).py_strip().is_empty() {
            networks.insert(evidence::network(&chain, &payload["network"]).ok()?);
        }
    }
    if networks.len() > 1 {
        return None;
    }
    let network = networks
        .into_iter()
        .next()
        .or_else(|| (chain == "bitcoin").then(|| "main".into()))?;
    let identity = if chain == "liquid" {
        let mut ids = BTreeSet::new();
        let mut displays = BTreeSet::new();
        for payload in maps.iter().copied() {
            for component in
                std::iter::once(payload).chain(payload.get("component").filter(|v| v.is_object()))
            {
                if !component["asset_id"].is_null() && component["asset_id"] != "" {
                    ids.insert(strict_txid(&component["asset_id"])?);
                }
                if !component["asset"].is_null() && component["asset"] != "" {
                    displays.insert(
                        text(&component["asset"])
                            .py_strip()
                            .py_upper()
                            .replace("L-BTC", "LBTC"),
                    );
                }
            }
        }
        if ids.len() != 1 || displays.len() > 1 || displays.iter().any(|v| v != &asset) {
            return None;
        }
        let id = ids.into_iter().next()?;
        if asset != "LBTC" && asset != id.py_upper() {
            return None;
        }
        id
    } else {
        asset
    };
    Some((chain, network, txid, identity))
}

fn script_owners(
    snapshot: &Snapshot,
    script: &Value,
    scope: (&str, &str),
) -> Result<BTreeSet<String>, String> {
    ownership::Index(snapshot.owned_index.as_ref())
        .script_owners(script, &(scope.0.into(), scope.1.into()))
}
fn input_owners(
    snapshot: &Snapshot,
    entry: &Value,
    scope: (&str, &str),
) -> Result<BTreeSet<String>, String> {
    ownership::Index(snapshot.owned_index.as_ref())
        .input_owners(entry, &(scope.0.into(), scope.1.into()))
}
/// Test-only isolated prerequisite projection, using the same raw snapshot facts.
/// This does not expose an implemented accounting stage.
pub(crate) fn prerequisites(snapshot: &Snapshot) -> Result<Value, String> {
    let rows: Vec<Value> = snapshot.transactions.iter().map(|row| json!(row)).collect();
    let index = ownership::Index(snapshot.owned_index.as_ref());
    let mut graphs = Vec::new();
    let mut scopes = Vec::new();
    let mut lookups = Vec::new();
    for row in &rows {
        let parsed = parse(&row["raw_json"])?;
        let scope = ownership::scope(row, parsed.as_ref(), index)?;
        let mut entries = Vec::new();
        if let Some(graph) = &parsed {
            for entry in graph["inputs"].as_array().unwrap() {
                let outpoint = text(&entry["outpoint"]);
                let txid = outpoint.split(':').next().unwrap_or("");
                let physical = scope.as_ref().map(|s| (s.0.clone(), s.1.clone()));
                entries.push(json!({"outpoint":index.outpoint(&entry["outpoint"],physical.as_ref()),"txid_wallets":index.txid(&json!(txid),physical.as_ref())}));
            }
        }
        graphs.push(parsed);
        scopes.push(scope);
        lookups.push(entries);
    }
    Ok(
        json!({"graphs":graphs,"scopes":scopes,"index_lookups":lookups,"profile_parsed":ownership::profile_parsed(&rows,index)?}),
    )
}
fn row_integer(row: &Value, key: &str) -> Result<BigInt, String> {
    row_integer_of(row, key)
}
fn row_integer_of<R: crate::fields::Fields + ?Sized>(row: &R, key: &str) -> Result<BigInt, String> {
    crate::interpretation::integer_or_zero(row.at(key)).map(|n| graph_integer(&n))
}
fn parsed_integer(value: &kassiber_pyjson::Value) -> Option<BigInt> {
    if let kassiber_pyjson::Value::Int(n) = value {
        Some(n.clone())
    } else {
        None
    }
}
fn bitcoin_fee(graph: &kassiber_pyjson::Value) -> Option<BigInt> {
    let inputs = stored::list(stored::get(graph, "inputs"));
    let outputs = stored::list(stored::get(graph, "outputs"));
    if stored::truth(stored::get(graph, "evidence_conflicts"))
        || inputs.is_empty()
        || outputs.is_empty()
    {
        return None;
    }
    let total = |legs: &[kassiber_pyjson::Value]| -> Option<BigInt> {
        legs.iter()
            .map(|leg| parsed_integer(stored::get(leg, "value_sats")))
            .sum()
    };
    let fee = total(inputs)? - total(outputs)?;
    (fee >= BigInt::from(0)).then(|| fee * 1000)
}
/// After scope recovery, admit the fee before walking wallet-delta ownership.
pub(crate) fn native_wallet_delta_msat(
    row: &Value,
    snapshot: &Snapshot,
    wallet: &str,
) -> Result<Option<BigInt>, String> {
    if snapshot.owned_index.is_none() || !evidence::current_observation(row)? {
        return Ok(None);
    }
    let parsed = stored::parse(&row["raw_json"])?;
    let projected = parsed.as_ref().map(stored::project);
    let scope = ownership::scope(
        row,
        projected.as_ref(),
        ownership::Index(snapshot.owned_index.as_ref()),
    )?;
    let (Some(graph), Some(scope)) = (parsed, scope) else {
        return Ok(None);
    };
    if scope.0 != "bitcoin" || scope.3 != "BTC" {
        return Ok(None);
    }
    let Some(fee) = bitcoin_fee(&graph) else {
        return Ok(None);
    };
    let raw = stored::object(&row["raw_json"]);
    let payload = if matches!(stored::get(&raw, "tx"), kassiber_pyjson::Value::Dict(_)) {
        stored::get(&raw, "tx")
    } else {
        &raw
    };
    let given_fee = stored::get(payload, "fee");
    if !matches!(given_fee, kassiber_pyjson::Value::Null)
        && !stored::equal(given_fee, &kassiber_pyjson::Value::Int(&fee / 1000), false)
    {
        return Ok(None);
    }
    let mut delta = BigInt::from(0);
    for (sign, side) in [(1, "inputs"), (-1, "outputs")] {
        for leg in stored::list(stored::get(&graph, side)) {
            if !stored::truth(stored::get(leg, "script")) {
                return Ok(None);
            }
            let value = parsed_integer(stored::get(leg, "value_sats"))
                .expect("fee admitted complete values");
            let owners = if sign == 1 {
                if !stored::truth(stored::get(leg, "outpoint")) {
                    return Ok(None);
                }
                input_owners(snapshot, &stored::project(leg), (&scope.0, &scope.1))?
            } else {
                script_owners(
                    snapshot,
                    &stored::project(stored::get(leg, "script")),
                    (&scope.0, &scope.1),
                )?
            };
            if owners.contains(wallet) {
                if owners.len() != 1 {
                    return Ok(None);
                }
                delta += value * (sign * 1000);
            }
        }
    }
    Ok(Some(delta))
}
pub(crate) fn native_zero_delta_ids<R: crate::fields::Row>(
    rows: &[R],
    snapshot: &Snapshot,
) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    for row in rows {
        if row.at("direction") != "outbound"
            || row_integer_of(row, "amount")? != BigInt::from(0)
            || row_integer_of(row, "fee")? != BigInt::from(0)
        {
            continue;
        }
        if native_wallet_delta_msat(&row.json(), snapshot, &text(row.at("wallet_id")))?
            == Some(BigInt::from(0))
        {
            ids.insert(text(row.at("id")));
        }
    }
    Ok(ids)
}
/// Python dict lookup hashes its key before checking values. Truthy JSON
/// containers are unhashable; numeric scripts simply cannot match string keys.
fn observed_script_key(value: &kassiber_pyjson::Value) -> Result<Option<String>, String> {
    use kassiber_pyjson::Value as Py;
    if !stored::truth(value) {
        return Ok(Some(String::new()));
    }
    match value {
        Py::List(_) | Py::Dict(_) => Err("TypeError".into()),
        Py::Str(s) => Ok(s.as_str().map(str::to_owned)),
        _ => Ok(None),
    }
}
pub fn normalize_owned_fees(rows: &mut [Value]) -> Result<(), String> {
    // Groups retain first-seen order, including which invalid group refuses first.
    let mut groups: Vec<(EventKey, Vec<usize>)> = Vec::new();
    let mut group_indices = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        if row["_kassiber_verified_chain_observation"].is_object() {
            let key = evidence::event_key(row)?;
            if key.chain == "bitcoin" && key.native_namespace == "chain" {
                let position = *group_indices.entry(key.clone()).or_insert_with(|| {
                    groups.push((key, Vec::new()));
                    groups.len() - 1
                });
                groups[position].1.push(i);
            }
        }
    }
    for (_, group) in groups {
        let mut outbound = Vec::new();
        for &i in &group {
            if rows[i]["direction"] == "outbound"
                && row_integer(&rows[i], "amount")? > BigInt::from(0)
            {
                outbound.push(i);
            }
        }
        if outbound.is_empty() {
            continue;
        }
        let mut eligible = true;
        for &i in &outbound {
            if rows[i]["observation_fee_attribution"] != "implicit_wallet_delta"
                || row_integer(&rows[i], "fee")? != BigInt::from(0)
                || truth(&rows[i]["amount_includes_fee"])
            {
                eligible = false;
                break;
            }
        }
        if !eligible
            || group
                .iter()
                .map(|i| text(&rows[*i]["wallet_id"]))
                .collect::<BTreeSet<_>>()
                .len()
                != group.len()
        {
            continue;
        }
        let raw: Vec<_> = group
            .iter()
            .map(|i| stored::object(&rows[*i]["raw_json"]))
            .collect();
        let mut scripts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (i, raw) in group.iter().zip(&raw) {
            for script in stored::list(stored::get(raw, "observer_owned_scripts")) {
                if let kassiber_pyjson::Value::Str(s) = script {
                    if let Some(s) = s.as_str().filter(|s| !s.is_empty()) {
                        scripts
                            .entry(s.into())
                            .or_default()
                            .insert(text(&rows[*i]["wallet_id"]));
                    }
                }
            }
        }
        let mut parsed = Vec::new();
        // Parse every observation before checking for a missing graph, as Python does.
        for &i in &group {
            parsed.push(stored::parse(&rows[i]["raw_json"])?);
        }
        let Some(graphs): Option<Vec<_>> = parsed.into_iter().collect() else {
            continue;
        };
        let shapes: BTreeSet<(Vec<String>, Vec<BigInt>)> = graphs
            .iter()
            .map(|g| {
                let mut ins: Vec<_> = stored::list(stored::get(g, "inputs"))
                    .iter()
                    .map(|v| stored::text(stored::get(v, "outpoint")))
                    .collect();
                ins.sort();
                let mut outs: Vec<_> = stored::list(stored::get(g, "outputs"))
                    .iter()
                    .map(|v| parsed_integer(stored::get(v, "n")).expect("parsed graph index"))
                    .collect();
                outs.sort();
                (ins, outs)
            })
            .collect();
        let Some(graph) = stored::merge(&graphs) else {
            continue;
        };
        let inputs = stored::list(stored::get(&graph, "inputs"));
        let outputs = stored::list(stored::get(&graph, "outputs"));
        if shapes.len() != 1
            || stored::truth(stored::get(&graph, "evidence_conflicts"))
            || inputs.is_empty()
            || outputs.is_empty()
            || inputs
                .iter()
                .any(|v| !stored::truth(stored::get(v, "outpoint")))
        {
            continue;
        }
        let mut input_totals: BTreeMap<String, BigInt> = BTreeMap::new();
        let mut output_totals: BTreeMap<String, BigInt> = BTreeMap::new();
        let mut total_in = BigInt::from(0);
        let mut total_out = BigInt::from(0);
        let mut valid = true;
        for vin in inputs {
            let key = observed_script_key(stored::get(vin, "script"))?;
            let owners = key.as_ref().and_then(|s| scripts.get(s));
            let value = parsed_integer(stored::get(vin, "value_sats"));
            if owners.is_none_or(|o| o.len() != 1) || value.is_none() {
                valid = false;
                break;
            }
            let value = value.unwrap();
            let owner = owners.unwrap().first().unwrap();
            *input_totals.entry(owner.clone()).or_default() += &value;
            total_in += value;
        }
        for vout in outputs {
            let value = parsed_integer(stored::get(vout, "value_sats"));
            let key = observed_script_key(stored::get(vout, "script"))?;
            let owners = key.as_ref().and_then(|s| scripts.get(s));
            if value.is_none() || owners.is_some_and(|o| o.len() > 1) {
                valid = false;
                break;
            }
            let value = value.unwrap();
            total_out += &value;
            if let Some(owner) = owners.and_then(|o| o.first()) {
                *output_totals.entry(owner.clone()).or_default() += value;
            }
        }
        let fee_sats = &total_in - &total_out;
        let fee = &fee_sats * 1000;
        if !valid
            || fee < BigInt::from(0)
            || raw.iter().any(|raw| {
                let value = stored::get(raw, "fee");
                !matches!(value, kassiber_pyjson::Value::Null)
                    && !stored::equal(value, &kassiber_pyjson::Value::Int(fee_sats.clone()), false)
            })
        {
            continue;
        }
        for &i in &group {
            let row = &rows[i];
            let wallet = text(&row["wallet_id"]);
            let zero = BigInt::from(0);
            let delta = (input_totals.get(&wallet).unwrap_or(&zero)
                - output_totals.get(&wallet).unwrap_or(&zero))
                * 1000;
            let signed = row_integer(row, "amount")?
                * if row["direction"] == "outbound" {
                    1
                } else {
                    -1
                };
            if delta != signed || row_integer(row, "fee")? != BigInt::from(0) {
                valid = false;
            }
        }
        if !valid {
            continue;
        }
        let amounts: BTreeMap<_, _> = outbound
            .iter()
            .map(|i| Ok((*i, row_integer(&rows[*i], "amount")?)))
            .collect::<Result<_, String>>()?;
        if amounts.values().sum::<BigInt>() < fee {
            continue;
        }
        let mut order = outbound;
        order.sort_by_key(|i| text(&rows[*i]["id"]));
        let bearer = order.iter().copied().find(|i| amounts[i] > fee);
        let mut remaining = fee.clone();
        for i in order {
            let allocated = if let Some(b) = bearer {
                if i == b {
                    fee.clone()
                } else {
                    BigInt::from(0)
                }
            } else {
                let v = remaining.clone().min(amounts[&i].clone());
                remaining -= &v;
                v
            };
            rows[i]["amount"] = integer_value(&(&amounts[&i] - &allocated));
            rows[i]["fee"] = integer_value(&allocated);
            rows[i]["observation_fee_attribution"] = json!("exact");
            rows[i]["custody_native_fee_normalized"] = json!(true);
        }
    }
    Ok(())
}

type ChainScope = (String, String, String, String);

pub(crate) fn reconcile_receipts<R: crate::fields::Row>(
    rows: &[R],
    snapshot: &Snapshot,
) -> Result<Value, String> {
    // A group reconciles only receipts of two or more wallets; with at most
    // one receiving wallet in the book no group can, and scopes are pure.
    let mut receiving = BTreeSet::new();
    for row in rows.iter().filter(|r| r.at("direction") == "inbound") {
        receiving.insert(text(row.at("wallet_id")));
        if receiving.len() > 1 {
            break;
        }
    }
    if receiving.len() <= 1 {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    let mut grouped: Vec<(ChainScope, Vec<&R>)> = Vec::new();
    let mut index = crate::fxhash::HashMap::default();
    for (row, scope) in rows.iter().map(|row| (row, row.onchain_scope())) {
        if let Some(scope) = scope {
            codec::push_grouped(&mut grouped, &mut index, scope, row);
        }
    }
    let mut result = serde_json::Map::new();
    for (scope, group) in grouped {
        if scope.3 != "BTC" {
            continue;
        }
        let inbound: BTreeSet<String> = group
            .iter()
            .filter(|r| r.at("direction") == "inbound")
            .map(|r| text(r.at("wallet_id")))
            .collect();
        if inbound.len() <= 1 {
            continue;
        }
        let wallets: BTreeSet<String> = group.iter().map(|r| text(r.at("wallet_id"))).collect();
        let parsed: Vec<Option<Value>> = group
            .iter()
            .map(|r| parse(r.at("raw_json")))
            .collect::<Result<_, _>>()?;
        let outputs: Vec<Value> = parsed
            .iter()
            .flatten()
            .next()
            .and_then(|g| g["outputs"].as_array())
            .cloned()
            .unwrap_or_default();
        let owners: Vec<Vec<Value>> = outputs
            .iter()
            .map(|o| {
                snapshot
                    .receipt_sources
                    .iter()
                    .filter(|s| {
                        s.get("chain") == Some(&json!(scope.0))
                            && s.get("network") == Some(&json!(scope.1))
                            && s.get("script_pubkey") == Some(&o["script"])
                            && wallets.contains(&text(s.get("wallet_id").unwrap_or(&Value::Null)))
                    })
                    .cloned()
                    .map(|row| Value::Object(row.into_map()))
                    .collect()
            })
            .collect();
        let overlap = owners.iter().any(|sources| {
            sources
                .iter()
                .map(|s| text(&s["wallet_id"]))
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        });
        let valued = !outputs.is_empty() && outputs.iter().all(|o| !o["value_sats"].is_null());
        let observed: BigInt = group
            .iter()
            .filter(|r| r.at("direction") == "inbound")
            .try_fold(BigInt::from(0), |sum, r| {
                Ok::<_, String>(sum + graph_integer(&codec::integer_or_zero(r.at("amount"))?))
            })?;
        let covered: BigInt = outputs
            .iter()
            .zip(&owners)
            .filter(|(_, s)| valued && !s.is_empty())
            .map(|(o, _)| graph_integer(&o["value_sats"]) * 1000)
            .sum();
        let kinds: BTreeSet<String> = group.iter().map(|r| text(r.at("wallet_kind"))).collect();
        let historical = kinds.contains("address")
            && ["descriptor", "xpub", "silent-payment"]
                .iter()
                .any(|k| kinds.contains(*k))
            && valued
            && covered > BigInt::from(0)
            && observed > covered;
        if !overlap && !historical {
            continue;
        }
        for row in &group {
            result.insert(text(row.at("id")), Value::Null);
        }
        if group
            .iter()
            .any(|r| r.at("direction") != "inbound" || int(r.at("fee")) != 0)
            || wallets.len() != group.len()
            || parsed.iter().any(Option::is_none)
            || snapshot.owned_index.is_none()
        {
            continue;
        }
        let mut authoritative = true;
        for row in &group {
            authoritative &= truth(&evidence::authority(&row.json())?["authoritative"]);
        }
        if !authoritative {
            continue;
        }
        let first = parsed[0].as_ref().unwrap();
        if parsed.iter().flatten().any(|g| {
            g["inputs"] != first["inputs"]
                || g["outputs"] != first["outputs"]
                || g["evidence_conflicts"] != first["evidence_conflicts"]
        }) || truth(&first["evidence_conflicts"])
            || outputs.iter().any(|o| o["value_sats"].is_null())
            || !truth(&first["inputs"])
        {
            continue;
        }
        let mut inputs_owned_or_incomplete = false;
        for e in first["inputs"].as_array().unwrap() {
            if !truth(&e["script"])
                || !truth(&e["outpoint"])
                || e["value_sats"].is_null()
                || !input_owners(snapshot, e, (&scope.0, &scope.1))?.is_empty()
            {
                inputs_owned_or_incomplete = true;
                break;
            }
        }
        if inputs_owned_or_incomplete {
            continue;
        }
        let mut full: BTreeMap<String, BigInt> = wallets
            .iter()
            .cloned()
            .map(|w| (w, BigInt::from(0)))
            .collect();
        let mut canonical = full.clone();
        let mut assignments = Vec::new();
        for (output, sources) in outputs.iter().zip(&owners) {
            if sources.is_empty() {
                continue;
            }
            let amount = graph_integer(&output["value_sats"]) * 1000;
            let mut by_wallet: BTreeMap<String, Value> = BTreeMap::new();
            for source in sources {
                let wallet = text(&source["wallet_id"]);
                let priority = match text(&source["source"]).as_str() {
                    "inventory" | "descriptor_config" => 0,
                    "address_list" => 1,
                    "sync_target" => 2,
                    _ => 99,
                };
                let entry=by_wallet.entry(wallet).or_insert_with(||json!({"id":source["wallet_id"],"label":source["wallet_label"],"kind":source["wallet_kind"],"deprecated":source["deprecated"],"active_transaction_count":source["active_transaction_count"],"priority":99}));
                entry["priority"] = json!(int(&entry["priority"]).min(priority));
            }
            for w in by_wallet.keys() {
                *full.get_mut(w).unwrap() += &amount;
            }
            let selected = by_wallet
                .values()
                .min_by_key(|v| {
                    (
                        truth(&v["deprecated"]),
                        !["descriptor", "xpub", "silent-payment"]
                            .contains(&text(&v["kind"]).as_str()),
                        int(&v["priority"]),
                        !truth(&v["active_transaction_count"]),
                        text(&v["label"]),
                        text(&v["id"]),
                    )
                })
                .unwrap();
            let owner = text(&selected["id"]);
            *canonical.get_mut(&owner).unwrap() += &amount;
            assignments.push(
                json!({"vout":output["n"],"wallet_id":owner,"amount_msat":integer_value(&amount)}),
            );
        }
        if group.iter().try_fold(false, |bad, r| {
            let w = text(r.at("wallet_id"));
            Ok::<_, String>(
                bad || canonical[&w] <= BigInt::from(0)
                    || ![&full[&w], &canonical[&w]]
                        .contains(&&graph_integer(&codec::integer_or_zero(r.at("amount"))?)),
            )
        })? {
            continue;
        }
        for row in &group {
            result.insert(text(row.at("id")),json!({"schema_version":1,"amount_msat":integer_value(&canonical[&text(row.at("wallet_id"))]),"outputs":assignments}));
        }
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod semantic_tests {
    use super::*;

    #[test]
    fn embedded_json_readers_keep_conversion_and_mutation_isolation() {
        let encoded = json!(r#"{"chain":NaN,"network":Infinity,"tx":{"txid":"sample"}}"#);
        for transfer_first in [false, true] {
            codec::clear_memos();
            if transfer_first {
                assert_eq!(transfer_object(&encoded)["chain"], "nan");
            } else {
                assert!(codec::object(&encoded)["chain"].is_null());
            }
            assert!(codec::object(&encoded)["network"].is_null());
            assert_eq!(transfer_object(&encoded)["network"], "inf");
            let mut owned = transfer_object(&encoded);
            owned["tx"]["txid"] = json!("changed");
            assert_eq!(transfer_object(&encoded)["tx"]["txid"], "sample");
            let mut owned = codec::object(&encoded);
            owned["tx"]["txid"] = json!("changed");
            assert_eq!(codec::object(&encoded)["tx"]["txid"], "sample");
        }
        let original = json!({"tx":{"txid":"sample"}});
        let mut owned = transfer_object(&original);
        owned["tx"]["txid"] = json!("changed");
        assert_eq!(original["tx"]["txid"], "sample");
        codec::clear_memos();
    }

    #[test]
    fn raw_fields_match_decoded_objects() {
        let texts = [
            r#"{"chain":"liquid","chain":"bitcoin","txid":"a","n":NaN}"#,
            r#"{"tx":{"txid":"b"},"network":Infinity,"x":[1,2.5]}"#,
            r#"[{"chain":"bitcoin"}]"#,
            r#""chain""#,
            r#"{"chain":"\ud800","network":"main"}"#,
            "not json",
            "",
        ];
        for text in texts {
            for decoded_first in [false, true] {
                codec::clear_memos();
                let value = json!(text);
                let object = transfer_object_ref(&value);
                if decoded_first {
                    let _ = &*object;
                }
                let keys = ["chain", "network", "txid", "tx", "n", "x", "missing"];
                let fields: Vec<Value> = keys
                    .iter()
                    .map(|key| object.field(key).into_owned())
                    .collect();
                for (key, field) in keys.iter().zip(fields) {
                    assert_eq!(
                        field,
                        transfer_object(&value)[*key],
                        "{text} {key} {decoded_first}"
                    );
                }
            }
            // A loaded text stands for the same object as its string.
            codec::clear_memos();
            assert_eq!(
                *raw_object(&raw_text(text)),
                transfer_object(&json!(text)),
                "{text}"
            );
        }
        codec::clear_memos();
    }

    #[test]
    fn cpython_onchain_scopes() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_semantics.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v.get("scope_row").is_some()) {
            assert_eq!(
                json!(onchain_scope(&v["scope_row"])),
                v["onchain_scope"],
                "{}",
                v["case"]
            );
        }
    }
    #[test]
    fn cpython_graph_indexes() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_semantics.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v.get("raw").is_some()) {
            assert_eq!(
                parse(&v["raw"]).unwrap().unwrap(),
                v["graph"],
                "{}",
                v["case"]
            );
        }
    }

    fn graph_case(case: &Value) -> Result<Value, String> {
        let index = ownership::Index(case.get("index").filter(|v| !v.is_null()));
        let physical = case["scope"].as_array().map(|v| (text(&v[0]), text(&v[1])));
        Ok(match case["operation"].as_str().unwrap() {
            "native_delta" | "native_zero" | "normalize_fees" => {
                let snapshot: Snapshot = serde_json::from_value(case["input"].clone()).unwrap();
                let mut rows: Vec<Value> =
                    snapshot.transactions.iter().map(|row| json!(row)).collect();
                match case["operation"].as_str().unwrap() {
                    "native_delta" => json!(native_wallet_delta_msat(
                        &rows[0],
                        &snapshot,
                        &text(&rows[0]["wallet_id"])
                    )?
                    .as_ref()
                    .map(integer_value)),
                    "native_zero" => json!(native_zero_delta_ids(&rows, &snapshot)?),
                    _ => {
                        for row in &mut rows {
                            let authority = evidence::authority(row)?;
                            if truth(&authority["authoritative"]) {
                                row["_kassiber_verified_chain_observation"] = authority;
                            }
                        }
                        normalize_owned_fees(&mut rows)?;
                        for row in &mut rows {
                            row.as_object_mut()
                                .unwrap()
                                .remove("_kassiber_verified_chain_observation");
                        }
                        json!(rows)
                    }
                }
            }
            "parse" => json!(parse(&case["raw"])?),
            "merge" => json!(merge(case["graphs"].as_array().unwrap())),
            "scope" => json!(ownership::scope(&case["row"], Some(&case["graph"]), index)?),
            "parsed_scope" => json!(ownership::parsed_scope(
                &case["graph"],
                index,
                &case["row"]
            )?),
            "profile" => json!(ownership::profile_parsed(
                case["rows"].as_array().unwrap(),
                index
            )?),
            "outpoint" => json!(index.outpoint(&case["value"], physical.as_ref())),
            "txid" => json!(index.txid(&case["value"], physical.as_ref())),
            "input_owners" => {
                json!(index.input_owners(&case["entry"], physical.as_ref().unwrap())?)
            }
            "source_scope" => json!(index.source_scope(
                case["graph"]["inputs"].as_array().unwrap(),
                &text(&case["wallet"])
            )?),
            "normalize" => json!(ownership::normalized(&case["value"][0], &case["value"][1])),
            _ => panic!("unknown graph test operation"),
        })
    }
    fn graph_cases(group: &str) {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_graph_cases.json"
        ))
        .unwrap();
        let mut count = 0;
        for case in vectors.iter().filter(|v| v["group"] == group) {
            let expected = case
                .get("error")
                .map(|e| Err(text(e)))
                .unwrap_or_else(|| Ok(case["expected"].clone()));
            assert_eq!(graph_case(case), expected, "{}", case["case"]);
            count += 1;
        }
        assert!(count > 0);
    }
    #[test]
    fn graph_merge_conflicts() {
        graph_cases("merge");
    }
    #[test]
    fn graph_mapping_values() {
        graph_cases("mapping");
    }
    #[test]
    fn graph_mapping_nonfinite() {
        graph_cases("mapping_nonfinite");
    }
    #[test]
    fn graph_mapping_admission() {
        graph_cases("mapping_admission");
    }
    #[test]
    fn graph_arithmetic_native() {
        graph_cases("arithmetic_native");
    }
    #[test]
    fn graph_arithmetic_fees() {
        graph_cases("arithmetic_fees");
    }
    #[test]
    fn graph_scope_recovery() {
        graph_cases("scope");
    }
    #[test]
    fn graph_index_precedence() {
        graph_cases("index");
    }
    #[test]
    fn graph_profile_merge() {
        graph_cases("profile");
    }
}
