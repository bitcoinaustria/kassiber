//! Authored-active component materialization. Invalid revisions remain visible
//! and keep their anchors blocked; no verdict is accepted from the snapshot.
mod anchors;
mod commitments;
mod conservation;
mod scope;
mod time;

use crate::codec::{self, pystr, text, truth, wide};
use crate::python_text::PythonText;
use crate::snapshot::{Component, Snapshot};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

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
fn group<K: PartialEq>(
    items: impl IntoIterator<Item = Value>,
    key: impl Fn(&Value) -> K,
) -> Vec<(K, Vec<Value>)> {
    let mut result: Vec<(K, Vec<Value>)> = Vec::new();
    for item in items {
        let k = key(&item);
        if let Some((_, ls)) = result.iter_mut().find(|(old, _)| *old == k) {
            ls.push(item);
        } else {
            result.push((k, vec![item]));
        }
    }
    result
}
fn integer(v: &Value) -> Result<Value, Error> {
    let kind = match v {
        Value::Null => Some("NoneType"),
        Value::Array(_) => Some("list"),
        Value::Object(_) => Some("dict"),
        _ => None,
    };
    if let Some(kind) = kind {
        return Err(Error {code:"TypeError",message:format!("int() argument must be a string, a bytes-like object or a real number, not '{kind}'")});
    }
    codec::integer(v).map_err(Error::value)
}
fn project(
    row: &Map<String, Value>,
    fields: &str,
    ints: &[&str],
    nullable: &[&str],
) -> Result<Value, Error> {
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
fn lineage(snapshot: &Snapshot, h: &Value) -> Vec<Value> {
    let mut headers: BTreeMap<String, Value> = snapshot
        .components
        .iter()
        .map(|c| (text(&c.header["id"]), Value::Object(c.header.clone())))
        .collect();
    headers.extend(
        snapshot
            .revision_headers
            .iter()
            .map(|r| (text(&r["id"]), Value::Object(r.clone()))),
    );
    let mut competing: Vec<_> = headers
        .values()
        .filter(|o| {
            o["profile_id"] == h["profile_id"]
                && o["lineage_id"] == h["lineage_id"]
                && o["state"] == "active"
                && o["id"] != h["id"]
        })
        .collect();
    competing.sort_by_key(|o| (wide(&o["revision"]), pystr(&o["id"])));
    let mut issues = Vec::new();
    if h["state"] == "active" && !competing.is_empty() {
        issues.push(json!({"code":"active_lineage_conflict","message":"multiple authored active revisions exist in this lineage","conflicts":competing.iter().map(|o|json!({"component_id":o["id"],"revision":wide(&o["revision"])})).collect::<Vec<_>>()}));
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
        let wrong_order = (wide(&target["revision"]) - wide(&h["revision"])) * order <= 0;
        if wrong_scope || wrong_order {
            issues.push(json!({"code":"revision_link_invalid","field":field,"target_component_id":id,"target_revision":wide(&target["revision"])}));
        }
    }
    issues
}
fn membership(snapshot: &Snapshot, c: &Component) -> Vec<Value> {
    // COALESCE distinguishes a null anchor from an empty authored anchor.
    let anchor = |l: &Map<String, Value>| {
        if l["anchor_transaction_id"].is_null() {
            l["transaction_id"].clone()
        } else {
            l["anchor_transaction_id"].clone()
        }
    };
    let ids: BTreeSet<_> = c
        .legs
        .iter()
        .filter(|l| l.get("profile_id") == c.header.get("profile_id"))
        .map(|l| anchor(l).to_string())
        .collect();
    let mut conflicts = BTreeSet::new();
    for other in &snapshot.components {
        if other.header["state"] != "active" || other.header["id"] == c.header["id"] {
            continue;
        }
        for l in &other.legs {
            let tx = anchor(l);
            if l.get("profile_id") == c.header.get("profile_id")
                && !tx.is_null()
                && ids.contains(&tx.to_string())
            {
                conflicts.insert((pystr(&tx), pystr(&other.header["id"])));
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
    c: &Component,
    routes: &[Value],
    native: &[Value],
) -> Result<Value, Error> {
    let mut h = Value::Object(c.header.clone());
    let stored = legs(c)?;
    let allocs = allocations(c)?;
    let terms: Vec<_> = c
        .economic_terms
        .iter()
        .map(|r| {
            let mut r = r.clone();
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
    let commitments: Vec<_> = c
        .commitments
        .iter()
        .filter(|r| r.get("profile_id") == h.get("profile_id"))
        .map(|r| {
            let mut v = Value::Object(r.clone());
            v["ordinal"] = integer(&v["ordinal"])?;
            Ok(v)
        })
        .collect::<Result<_, Error>>()?;
    if !h["expected_evidence_count"].is_null() {
        h["expected_evidence_count"] = integer(&h["expected_evidence_count"])?;
    }
    let evidence = commitments::evidence_status(snapshot, &h, &stored, &commitments);
    if !truth(&evidence["valid"]) {
        issues.push(json!({"code":"component_evidence_commitment_invalid","message":"author-bound activation evidence is missing, incomplete, conflicted, or no longer matches local canonical anchors","status":evidence["status"],"expected":evidence["expected_count"],"actual":evidence["commitment_count"]}));
    }
    let native = commitments::native_status(&stored, &allocs, native);
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
    issues.extend(lineage(snapshot, &h));
    let conflicts = membership(snapshot, c);
    if !conflicts.is_empty() {
        issues.push(json!({"code":"active_transaction_membership_conflict","message":"a transaction belongs to another authored active component","conflicts":conflicts}));
    }
    let effective = if issues.is_empty() { "active" } else { "draft" };
    validation["activatable"] = json!(issues.is_empty());
    validation["issues"] = json!(issues);
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
    result.insert(
        "legs".into(),
        json!(stored
            .into_iter()
            .map(|mut l| {
                l.as_object_mut().unwrap().remove("location_ref");
                l
            })
            .collect::<Vec<_>>()),
    );
    result.insert("allocations".into(), json!(allocs));
    result.insert("economic_terms".into(), json!(terms));
    result.insert("validation".into(), validation);
    Ok(Value::Object(result))
}
pub fn compute(snapshot: &Snapshot) -> Result<Value, Error> {
    let profile = pystr(&snapshot.profile["id"]);
    let mut components: Vec<_> = snapshot
        .components
        .iter()
        .filter(|c| c.header["profile_id"] == profile && c.header["state"] == "active")
        .collect();
    components.sort_by_key(|c| {
        (
            pystr(&c.header["created_at"]),
            wide(&c.header["revision"]),
            pystr(&c.header["id"]),
        )
    });
    let mut quantity_legs = Vec::new();
    let mut quantity_allocs = Vec::new();
    for c in &components {
        if c.header["conservation_mode"] == "quantity" {
            quantity_legs.extend(legs(c)?);
            quantity_allocs.extend(allocations(c)?);
        }
    }
    for ls in [&mut quantity_legs, &mut quantity_allocs] {
        ls.sort_by_key(|l| {
            (
                pystr(&l["component_id"]),
                wide(&l["ordinal"]),
                pystr(&l["id"]),
            )
        });
    }
    let routes = scope::profile_routes(snapshot, &quantity_legs, &quantity_allocs)?;
    let native = if components.is_empty() {
        Vec::new()
    } else {
        commitments::native_context(snapshot)?
    };
    let components: Vec<_> = components
        .into_iter()
        .map(|c| materialize(snapshot, c, &routes, &native))
        .collect::<Result<_, _>>()?;
    let blockers:Vec<_>=components.iter().filter(|c|c["effective_state"]!="active").map(|c| {
        let codes:BTreeSet<_>=array(&c["validation"]["issues"]).iter().map(|i|if truth(&i["code"]) {text(&i["code"])} else {"unknown".into()}).collect();
        json!({"component_id":c["id"],"lineage_id":c["lineage_id"],"revision":c["revision"],"issue_codes":codes})
    }).collect();
    Ok(json!({"components":components,"blockers":blockers}))
}
