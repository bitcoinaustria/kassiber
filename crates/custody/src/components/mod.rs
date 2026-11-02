//! Authored-active component materialization. Invalid revisions remain visible
//! and keep their anchors blocked; no verdict is accepted from the snapshot.
mod anchors;
mod commitments;
mod conservation;
pub mod review;
mod scope;
pub(crate) mod time;

use crate::codec::{self, pystr, text, truth, wide};
use crate::python_text::PythonText;
use crate::quantity::{exact, number};
use kassiber_pyjson::BigInt;
fn big(v: &Value) -> BigInt {
    exact(v).expect("converted component integer")
}
use crate::snapshot::{Component, Record, Snapshot};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}
impl Error {
    fn value(message: impl Into<String>) -> Self {
        Self {
            code: "ValueError",
            message: message.into(),
        }
    }
    fn missing(key: &str) -> Self {
        Self {
            code: "KeyError",
            message: format!("'{key}'"),
        }
    }
}
impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::value(message)
    }
}
impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::value(message)
    }
}
impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.message
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.message)
    }
}
impl Error {
    pub(crate) fn encoding(message: String) -> Self {
        Self {
            code: "UnicodeEncodeError",
            message,
        }
    }
    // UnicodeEncodeError inherits ValueError in CPython.
    pub(crate) fn is_value_or_type(&self) -> bool {
        matches!(self.code, "TypeError" | "ValueError" | "UnicodeEncodeError")
    }
    fn is_evidence_unavailable(&self) -> bool {
        self.is_value_or_type() || matches!(self.code, "KeyError" | "sqlite3.Error")
    }
}
fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn absent(v: &Value) -> bool {
    v.is_null() || v == ""
}
fn or<'a>(a: &'a Value, b: &'a Value) -> &'a Value {
    if truth(a) {
        a
    } else {
        b
    }
}
fn owned(l: &Value) -> bool {
    matches!(l["role"].as_str(), Some("destination" | "retained"))
}
fn sink(l: &Value) -> bool {
    matches!(
        l["role"].as_str(),
        Some("destination" | "retained" | "fee" | "external" | "suspense")
    )
}
fn by_id<'a>(ls: &'a [Value], id: &Value) -> Option<&'a Value> {
    ls.iter().rev().find(|l| pystr(&l["id"]) == pystr(id))
}
fn location(l: &Value) -> (String, String, String) {
    (
        pystr(&l["wallet_id"]),
        text(&l["exposure"]),
        text(&l["conservation_unit"]),
    )
}
fn group<K: Eq + std::hash::Hash + Clone>(
    items: impl IntoIterator<Item = Value>,
    key: impl Fn(&Value) -> K,
) -> Vec<(K, Vec<Value>)> {
    let mut result = Vec::new();
    let mut index = HashMap::new();
    for item in items {
        let k = key(&item);
        codec::push_grouped(&mut result, &mut index, k, item);
    }
    result
}
pub(crate) fn integer(v: &Value) -> Result<Value, Error> {
    let bytes = codec::raw_bytes(v);
    let kind = match v {
        Value::Null => Some("NoneType"),
        Value::Array(_) => Some("list"),
        Value::Object(_) if bytes.is_none() => Some("dict"),
        _ => None,
    };
    if let Some(kind) = kind {
        return Err(Error {code:"TypeError",message:format!("int() argument must be a string, a bytes-like object or a real number, not '{kind}'")});
    }
    if matches!(codec::from_json(v), Ok(kassiber_pyjson::Value::Float(value)) if value.is_infinite())
    {
        return Err(Error {
            code: "OverflowError",
            message: "cannot convert float infinity to integer".into(),
        });
    }
    codec::integer(v).map_err(|message| {
        if v.is_string() || bytes.is_some() {
            let value = codec::from_json(v).expect("string scalar");
            Error::value(format!(
                "invalid literal for int() with base 10: {}",
                crate::interpretation::value_repr(&value)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ))
        } else {
            Error::value(message)
        }
    })
}
// SQLite orders raw INTEGER/REAL facts before TEXT. Do not int() here:
// malformed numeric text must be refused at the reference's conversion point.
fn raw_order(a: &Value, b: &Value) -> std::cmp::Ordering {
    if let (Some(a), Some(b)) = (codec::raw_bytes(a), codec::raw_bytes(b)) {
        return a.cmp(&b);
    }
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            match (
                a.to_string().parse::<kassiber_pyjson::BigInt>(),
                b.to_string().parse::<kassiber_pyjson::BigInt>(),
            ) {
                (Ok(a), Ok(b)) => a.cmp(&b),
                _ => a.as_f64().partial_cmp(&b.as_f64()).unwrap(),
            }
        }
        (Value::Number(_), _) => std::cmp::Ordering::Less,
        (_, Value::Number(_)) => std::cmp::Ordering::Greater,
        _ => pystr(a).cmp(&pystr(b)),
    }
}
fn project(row: &Record, fields: &str, ints: &[&str], nullable: &[&str]) -> Result<Value, Error> {
    let mut result = Map::new();
    for key in fields.split_whitespace() {
        let mut v = row.get(key).ok_or_else(|| Error::missing(key))?.clone();
        if ints.contains(&key) || (nullable.contains(&key) && !v.is_null()) {
            v = integer(&v)?;
        }
        result.insert(key.into(), v);
    }
    Ok(Value::Object(result))
}
const LEG_FIELDS:&str="id component_id workspace_id profile_id ordinal role rail chain network asset exposure conservation_unit amount_msat valuation_unit valuation_amount occurred_at transaction_id anchor_transaction_id wallet_id location_ref notes created_at";
const ALLOCATION_FIELDS:&str="id component_id workspace_id profile_id ordinal source_leg_id sink_leg_id source_amount_msat sink_amount_msat created_at";
fn legs(c: &Component) -> Result<Vec<Value>, Error> {
    c.legs
        .iter()
        .map(|r| {
            project(
                r,
                LEG_FIELDS,
                &["ordinal", "amount_msat"],
                &["valuation_amount"],
            )
        })
        .collect()
}
fn allocations(c: &Component) -> Result<Vec<Value>, Error> {
    c.allocations
        .iter()
        .map(|r| {
            project(
                r,
                ALLOCATION_FIELDS,
                &["ordinal", "source_amount_msat", "sink_amount_msat"],
                &[],
            )
        })
        .collect()
}
/// Active legs per (encoded leg profile, encoded anchor), as (owning header
/// id, anchor text, owning header id text).
type Members<'a> = HashMap<(Option<String>, String), Vec<(&'a Value, String, String)>>;
/// Anchor rows per anchor id text, with their position in `anchors` order.
type AnchorRows<'a> = HashMap<String, Vec<(usize, &'a Map<String, Value>)>>;
/// Lookups every materialization shares, built once per stage. Encoded JSON
/// keys are exact: serde's encoding of a Value is injective.
pub(super) struct Shared<'a> {
    headers: BTreeMap<String, Value>,
    /// Header keys per encoded lineage id, in `headers` order.
    lineages: HashMap<String, Vec<String>>,
    /// Active legs with a non-null anchor.
    members: Members<'a>,
    pub(super) anchors: AnchorRows<'a>,
}
impl<'a> Shared<'a> {
    fn new(snapshot: &'a Snapshot) -> Self {
        let mut headers: BTreeMap<String, Value> = snapshot
            .components
            .iter()
            .map(|c| {
                (
                    text(&c.header["id"]),
                    Value::Object(c.header.clone().into_map()),
                )
            })
            .collect();
        headers.extend(
            snapshot
                .revision_headers
                .iter()
                .map(|r| (text(&r["id"]), Value::Object(r.clone().into_map()))),
        );
        let mut lineages: HashMap<String, Vec<String>> = HashMap::new();
        for (key, o) in &headers {
            lineages
                .entry(o["lineage_id"].to_string())
                .or_default()
                .push(key.clone());
        }
        let mut members: HashMap<_, Vec<_>> = HashMap::new();
        for other in &snapshot.components {
            if other.header["state"] != "active" {
                continue;
            }
            for l in &other.legs {
                let tx = membership_anchor(l);
                if !tx.is_null() {
                    members
                        .entry((l.get("profile_id").map(Value::to_string), tx.to_string()))
                        .or_default()
                        .push((&other.header["id"], pystr(&tx), pystr(&other.header["id"])));
                }
            }
        }
        let mut anchors: HashMap<String, Vec<_>> = HashMap::new();
        for (at, row) in snapshot
            .anchors
            .values()
            .filter_map(Value::as_object)
            .enumerate()
        {
            anchors
                .entry(pystr(&row["id"]))
                .or_default()
                .push((at, row));
        }
        Self {
            headers,
            lineages,
            members,
            anchors,
        }
    }
}
fn lineage(shared: &Shared, h: &Value) -> Result<Vec<Value>, Error> {
    let headers = &shared.headers;
    // Only headers of this lineage can pass the lineage equality below.
    let mut competing: Vec<_> = shared
        .lineages
        .get(&h["lineage_id"].to_string())
        .into_iter()
        .flatten()
        .map(|key| &headers[key])
        .filter(|o| {
            o["profile_id"] == h["profile_id"]
                && o["lineage_id"] == h["lineage_id"]
                && o["state"] == "active"
                && o["id"] != h["id"]
        })
        .collect();
    competing.sort_by(|a, b| {
        raw_order(&a["revision"], &b["revision"])
            .then_with(|| pystr(&a["id"]).cmp(&pystr(&b["id"])))
    });
    let mut issues = Vec::new();
    if h["state"] == "active" && !competing.is_empty() {
        issues.push(json!({"code":"active_lineage_conflict","message":"multiple authored active revisions exist in this lineage","conflicts":competing.iter().map(|o|Ok(json!({"component_id":o["id"],"revision":integer(&o["revision"])?}))).collect::<Result<Vec<_>,Error>>()?}));
    }
    for (field, order) in [
        ("supersedes_component_id", -1),
        ("superseded_by_component_id", 1),
    ] {
        let id = &h[field];
        if id.is_null() {
            continue;
        }
        let Some(target) = headers.get(&pystr(id)) else {
            issues.push(
                json!({"code":"revision_link_missing","field":field,"target_component_id":id}),
            );
            continue;
        };
        let wrong_scope =
            target["profile_id"] != h["profile_id"] || target["lineage_id"] != h["lineage_id"];
        let target_revision = integer(&target["revision"])?;
        let wrong_order = (wide(&target_revision) - wide(&integer(&h["revision"])?)) * order <= 0;
        if wrong_scope || wrong_order {
            issues.push(json!({"code":"revision_link_invalid","field":field,"target_component_id":id,"target_revision":target_revision}));
        }
    }
    Ok(issues)
}
// COALESCE distinguishes a null anchor from an empty authored anchor.
fn membership_anchor(l: &Record) -> Value {
    if l["anchor_transaction_id"].is_null() {
        l["transaction_id"].clone()
    } else {
        l["anchor_transaction_id"].clone()
    }
}
fn membership(shared: &Shared, c: &Component) -> Vec<Value> {
    let ids: BTreeSet<_> = c
        .legs
        .iter()
        .filter(|l| l.get("profile_id") == c.header.get("profile_id"))
        .map(|l| membership_anchor(l).to_string())
        .collect();
    let profile = c.header.get("profile_id").map(Value::to_string);
    let mut conflicts = BTreeSet::new();
    // Active legs of this profile with one of these non-null anchors.
    for id in &ids {
        for (owner, tx, other) in shared
            .members
            .get(&(profile.clone(), id.clone()))
            .into_iter()
            .flatten()
        {
            if *owner != &c.header["id"] {
                conflicts.insert((tx.clone(), other.clone()));
            }
        }
    }
    conflicts
        .into_iter()
        .map(|(tx, id)| json!({"transaction_id":tx,"component_id":id}))
        .collect()
}
fn materialize(
    snapshot: &Snapshot,
    shared: &Shared,
    c: &Component,
    routes: &[Value],
    native: &commitments::Native,
) -> Result<Value, Error> {
    let mut h = Value::Object(c.header.clone().into_map());
    let stored = legs(c)?;
    let allocs = allocations(c)?;
    let terms: Vec<_> = c
        .economic_terms
        .iter()
        .map(|r| {
            let mut r = r.clone().into_map();
            for k in [
                "reviewed_source_amount_msat",
                "swap_fee_msat",
                "payout_amount_msat",
            ] {
                if let Some(v) = r.get(k).filter(|v| !v.is_null()) {
                    r.insert(k.into(), integer(v)?);
                }
            }
            Ok(Value::Object(r))
        })
        .collect::<Result<_, Error>>()?;
    let mode = h["conservation_mode"]
        .as_str()
        .filter(|s| !s.py_strip().is_empty())
        .ok_or_else(|| Error {
            code: "custody_component_validation",
            message: "conservation_mode is required".into(),
        })?
        .py_strip();
    if !mode.bytes().enumerate().all(|(i, b)| {
        if i == 0 {
            b.is_ascii_lowercase()
        } else {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.:-".contains(&b)
        }
    }) {
        return Err(Error {
            code: "custody_component_validation",
            message: "conservation_mode must be a lowercase stable token".into(),
        });
    }
    if !["quantity", "conversion"].contains(&mode) {
        return Err(Error {
            code: "custody_component_validation",
            message: "conservation_mode is not supported".into(),
        });
    }
    let mut validation = conservation::validate(&stored, &allocs, &h)?;
    let mut issues = array(&validation["issues"]).to_vec();
    let counts = [
        (
            "expected_leg_count",
            stored.len(),
            "component_leg_count_mismatch",
        ),
        (
            "expected_allocation_count",
            allocs.len(),
            "component_allocation_count_mismatch",
        ),
        (
            "expected_economic_term_count",
            terms.len(),
            "component_economic_term_count_mismatch",
        ),
    ];
    if counts.iter().any(|(k, _, _)| h[k].is_null()) {
        issues.push(json!({"code":"component_content_commitment_missing","message":"this legacy component header does not commit to its child row counts; create a new revision before activation"}));
    } else {
        for (k, n, code) in counts {
            h[k] = integer(&h[k])?;
            if wide(&h[k]) != n as i128 {
                issues.push(json!({"code":code,"expected":h[k],"actual":n}));
            }
        }
    }
    if !h["expected_evidence_count"].is_null() {
        h["expected_evidence_count"] = integer(&h["expected_evidence_count"])?;
    }
    let commitments: Vec<_> = c
        .commitments
        .iter()
        .filter(|r| r.get("profile_id") == h.get("profile_id"))
        .map(|r| {
            let mut v = Value::Object(r.clone().into_map());
            if !h["expected_evidence_count"].is_null() {
                v["ordinal"] = integer(&v["ordinal"])?;
            }
            Ok(v)
        })
        .collect::<Result<_, Error>>()?;
    let evidence = if h["state"] == "active" {
        commitments::evidence_status(shared, &h, &stored, &commitments)?
    } else {
        json!({"valid":true,"status":"not_required_until_activation","expected_count":h["expected_evidence_count"],"commitment_count":0})
    };
    if !truth(&evidence["valid"]) {
        issues.push(json!({"code":"component_evidence_commitment_invalid","message":"author-bound activation evidence is missing, incomplete, conflicted, or no longer matches local canonical anchors","status":evidence["status"],"expected":evidence["expected_count"],"actual":evidence["commitment_count"]}));
    }
    let native = if h["state"] == "active" {
        commitments::native_status(&stored, &allocs, native)?
    } else {
        json!({"status":"unverified","usable":true,"boundary_count":0,"supported_boundary_count":0,"partially_supported_boundary_count":0,"contradicted_boundary_count":0})
    };
    if !truth(&native["usable"]) {
        issues.push(json!({"code":"component_native_support_contradicted","message":"recovered same-native-event policy evidence conflicts with an authored component boundary","supported_boundaries":native["supported_boundary_count"],"contradicted_boundaries":native["contradicted_boundary_count"]}));
    }
    let anchors = anchors::validate(
        snapshot,
        &stored,
        &allocs,
        &terms,
        &text(&h["conservation_mode"]),
        routes,
    )?;
    issues.extend(array(&anchors["issues"]).iter().cloned());
    if !array(&anchors["warnings"]).is_empty() {
        validation["warnings"] = anchors["warnings"].clone();
    }
    validation["anchors"] = anchors;
    issues.extend(lineage(shared, &h)?);
    let conflicts = if h["state"] == "active" {
        membership(shared, c)
    } else {
        Vec::new()
    };
    if !conflicts.is_empty() {
        issues.push(json!({"code":"active_transaction_membership_conflict","message":"a transaction belongs to another authored active component","conflicts":conflicts}));
    }
    let effective = if issues.is_empty() { "active" } else { "draft" };
    validation["activatable"] = json!(issues.is_empty());
    validation["issues"] = Value::Array(issues);
    for k in [
        "revision",
        "expected_leg_count",
        "expected_allocation_count",
        "expected_economic_term_count",
        "expected_evidence_count",
    ] {
        if k == "revision" || !h[k].is_null() {
            h[k] = integer(&h[k])?;
        }
    }
    let mut result = Map::new();
    for k in "id lineage_id workspace_id profile_id revision component_type conservation_mode state evidence_kind evidence_grade conversion_policy expected_leg_count expected_allocation_count expected_economic_term_count expected_evidence_count notes change_reason supersedes_component_id superseded_by_component_id activated_at superseded_at created_at".split_whitespace() {result.insert(k.into(),h.get(k).ok_or_else(||Error::missing(k))?.clone());}
    result.insert("effective_state".into(), json!(effective));
    result.insert(
        "conversion_reviewed".into(),
        json!(truth(&h["conversion_reviewed"])),
    );
    result.insert(
        "authored_source".into(),
        or(&h["authored_source"], &json!("user")).clone(),
    );
    result.insert("evidence_status".into(), evidence);
    result.insert("native_support_status".into(), native);
    // The parts move into the result; `json!` would copy each one.
    result.insert(
        "legs".into(),
        Value::Array(
            stored
                .into_iter()
                .map(|mut l| {
                    l.as_object_mut().unwrap().remove("location_ref");
                    l
                })
                .collect(),
        ),
    );
    result.insert("allocations".into(), Value::Array(allocs));
    result.insert("economic_terms".into(), Value::Array(terms));
    result.insert("validation".into(), validation);
    Ok(Value::Object(result))
}
fn header_order(a: &Component, b: &Component) -> Result<std::cmp::Ordering, Error> {
    for key in ["created_at", "revision", "id"] {
        let (a, b) = (&a.header[key], &b.header[key]);
        if a == b {
            continue;
        }
        if (a.is_string() && b.is_string())
            || (a.is_number() && b.is_number())
            || (codec::raw_bytes(a).is_some() && codec::raw_bytes(b).is_some())
        {
            let order = raw_order(a, b);
            if !order.is_eq() {
                return Ok(order);
            }
            continue;
        }
        let kind = |v: &Value| {
            if codec::raw_bytes(v).is_some() {
                "bytes"
            } else {
                match v {
                    Value::Null => "NoneType",
                    Value::Bool(_) => "bool",
                    Value::Number(n) if n.as_i64().is_some() || n.as_u64().is_some() => "int",
                    Value::Number(_) => "float",
                    Value::String(_) => "str",
                    Value::Array(_) => "list",
                    Value::Object(_) => "dict",
                }
            }
        };
        return Err(Error {
            code: "TypeError",
            message: format!(
                "'<' not supported between instances of '{}' and '{}'",
                kind(a),
                kind(b)
            ),
        });
    }
    Ok(std::cmp::Ordering::Equal)
}

pub fn compute(snapshot: &Snapshot) -> Result<Value, Error> {
    compute_with(snapshot, None)
}

/// `compute`, reusing evidence's enrichment where it is this stage's own.
pub fn compute_with(
    snapshot: &Snapshot,
    prepared: Option<&crate::evidence::Prepared>,
) -> Result<Value, Error> {
    let profile = pystr(&snapshot.profile["id"]);
    let mut components: Vec<_> = snapshot
        .components
        .iter()
        .filter(|c| c.header["profile_id"] == profile && c.header["state"] == "active")
        .collect();
    // SQL returns the normal population in this order already. Compare the
    // original Python tuple operands: stringifying a corrupt timestamp would
    // hide the pure repository's TypeError and continue materialization.
    for index in 1..components.len() {
        let mut at = index;
        while at > 0 && header_order(components[at], components[at - 1])?.is_lt() {
            components.swap(at, at - 1);
            at -= 1;
        }
    }
    // The SQL reference reads all quantity legs before any allocations, in
    // component/ordinal/id order; convert only after that raw ordering.
    let mut raw_legs = Vec::new();
    let mut raw_allocs = Vec::new();
    for c in &components {
        if c.header["conservation_mode"] == "quantity" {
            raw_legs.extend(c.legs.iter());
            raw_allocs.extend(c.allocations.iter());
        }
    }
    for rows in [&mut raw_legs, &mut raw_allocs] {
        rows.sort_by(|a, b| {
            pystr(&a["component_id"])
                .cmp(&pystr(&b["component_id"]))
                .then_with(|| raw_order(&a["ordinal"], &b["ordinal"]))
                .then_with(|| pystr(&a["id"]).cmp(&pystr(&b["id"])))
        });
    }
    let quantity_legs = raw_legs
        .iter()
        .map(|r| {
            project(
                r,
                LEG_FIELDS,
                &["ordinal", "amount_msat"],
                &["valuation_amount"],
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let quantity_allocs = if quantity_legs.is_empty() {
        Vec::new()
    } else {
        raw_allocs
            .iter()
            .map(|r| {
                project(
                    r,
                    ALLOCATION_FIELDS,
                    &["ordinal", "source_amount_msat", "sink_amount_msat"],
                    &[],
                )
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let span = crate::timing::span("components.routes");
    let routes = scope::profile_routes(snapshot, &quantity_legs, &quantity_allocs)?;
    drop(span);
    let span = crate::timing::span("components.native");
    let native = if components.is_empty() {
        Vec::new()
    } else {
        commitments::native_context(snapshot, prepared)?
    };
    let native = commitments::Native::new(native);
    drop(span);
    let _span = crate::timing::span("components.materialize");
    let shared = Shared::new(snapshot);
    let components: Vec<_> = components
        .into_iter()
        .map(|c| materialize(snapshot, &shared, c, &routes, &native))
        .collect::<Result<_, _>>()?;
    let blockers:Vec<_>=components.iter().filter(|c|c["effective_state"]!="active").map(|c| {
        let codes:BTreeSet<_>=array(&c["validation"]["issues"]).iter().map(|i|if truth(&i["code"]) {text(&i["code"])} else {"unknown".into()}).collect();
        json!({"component_id":c["id"],"lineage_id":c["lineage_id"],"revision":c["revision"],"issue_codes":codes})
    }).collect();
    let mut result = Map::new();
    result.insert("blockers".into(), Value::Array(blockers));
    result.insert("components".into(), Value::Array(components));
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn component_integer_refusal_messages_match_python() {
        for (raw, code, message) in [
            (
                "\"\"",
                "ValueError",
                "invalid literal for int() with base 10: ''",
            ),
            (
                "\"legacy scalar\"",
                "ValueError",
                "invalid literal for int() with base 10: 'legacy scalar'",
            ),
            (
                "1e999",
                "OverflowError",
                "cannot convert float infinity to integer",
            ),
            (
                "-1e999",
                "OverflowError",
                "cannot convert float infinity to integer",
            ),
        ] {
            let error = integer(&serde_json::from_str::<Value>(raw).unwrap()).unwrap_err();
            assert_eq!((error.code, error.message.as_str()), (code, message));
        }
        assert_eq!(integer(&json!(1.5)).unwrap(), json!(1));
    }

    #[test]
    fn component_header_sort_retains_python_type_errors() {
        let component = |created_at: Value| Component {
            header: serde_json::from_value(json!({"created_at":created_at,"revision":1,"id":"a"}))
                .unwrap(),
            legs: vec![],
            allocations: vec![],
            economic_terms: vec![],
            commitments: vec![],
        };
        for (raw, kind) in [
            ("null", "NoneType"),
            ("7", "int"),
            ("1.5", "float"),
            ("1e999", "float"),
        ] {
            let a = component(json!("2026-01-01"));
            let b = component(serde_json::from_str(raw).unwrap());
            let error = header_order(&a, &b).unwrap_err();
            assert_eq!(error.code, "TypeError");
            assert_eq!(
                error.message,
                format!("'<' not supported between instances of 'str' and '{kind}'")
            );
            let error = header_order(&b, &a).unwrap_err();
            assert_eq!(
                error.message,
                format!("'<' not supported between instances of '{kind}' and 'str'")
            );
        }
    }

    #[test]
    fn component_exception_catch_families() {
        for code in [
            "TypeError",
            "ValueError",
            "UnicodeEncodeError",
            "KeyError",
            "sqlite3.Error",
            "OverflowError",
            "RuntimeError",
        ] {
            let error = Error {
                code,
                message: "original detail".into(),
            };
            assert_eq!(
                error.is_value_or_type(),
                matches!(code, "TypeError" | "ValueError" | "UnicodeEncodeError"),
                "{code}"
            );
            assert_eq!(
                error.is_evidence_unavailable(),
                matches!(
                    code,
                    "TypeError"
                        | "ValueError"
                        | "UnicodeEncodeError"
                        | "KeyError"
                        | "sqlite3.Error"
                ),
                "{code}"
            );
            assert_eq!(serde_json::to_value(error).unwrap(), "original detail");
        }
    }
    #[test]
    fn component_raw_integer_order_is_exact() {
        assert!(raw_order(&json!(9007199254740992_i64), &json!(9007199254740993_i64)).is_lt());
    }
}

#[cfg(test)]
mod byte_tests {
    use super::*;
    #[test]
    fn component_bytes_sort_and_integer_semantics_match_python() {
        use kassiber_pyjson::Value as Py;
        let bytes = |b: &[u8]| codec::to_json(&Py::Bytes(b.to_vec()));
        assert_eq!(integer(&bytes(b"77")).unwrap(), json!(77));
        let error = integer(&bytes(&[255, 0])).unwrap_err();
        assert_eq!(error.code, "ValueError");
        assert_eq!(
            error.message,
            "invalid literal for int() with base 10: b'\\xff\\x00'"
        );
        assert_eq!(
            raw_order(&bytes(b"\0"), &bytes(b"'")),
            std::cmp::Ordering::Less
        );
        let component = |created: Value| Component {
            header: serde_json::from_value(json!({"created_at":created,"revision":1,"id":"a"}))
                .unwrap(),
            legs: vec![],
            allocations: vec![],
            economic_terms: vec![],
            commitments: vec![],
        };
        assert_eq!(
            header_order(&component(bytes(b"\0")), &component(bytes(b"'"))).unwrap(),
            std::cmp::Ordering::Less
        );
        let error = header_order(&component(bytes(b"a")), &component(json!("b"))).unwrap_err();
        assert_eq!(error.code, "TypeError");
        assert_eq!(
            error.message,
            "'<' not supported between instances of 'bytes' and 'str'"
        );
    }
}
