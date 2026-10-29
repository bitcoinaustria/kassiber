//! Author commitments use canonical evidence without joined observer authority,
//! exactly as the historical SQL reads do. Lifecycle changes are not verdicts.
use super::*;
use crate::evidence;
use kassiber_pyjson::{Key, Value as Py};

pub(super) fn without_provenance(row: &Map<String, Value>) -> Map<String, Value> {
    row.iter()
        .filter(|(k, _)| !k.starts_with("observation_"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
fn order(row: &Map<String, Value>) -> (String, String, String) {
    (
        pystr(&row["occurred_at"]),
        pystr(&row["created_at"]),
        pystr(&row["id"]),
    )
}
fn anchors(legs: &[Value]) -> BTreeSet<String> {
    legs.iter()
        .filter(|l| !absent(&l["anchor_transaction_id"]) || !absent(&l["transaction_id"]))
        .map(|l| pystr(or(&l["anchor_transaction_id"], &l["transaction_id"])))
        .collect()
}
fn current(shared: &Shared, h: &Value, legs: &[Value]) -> Result<Vec<Value>, Error> {
    let ids = anchors(legs);
    // Anchor rows with these ids, in anchor order.
    let mut candidates: Vec<_> = ids
        .iter()
        .filter_map(|id| shared.anchors.get(id))
        .flatten()
        .collect();
    candidates.sort_unstable_by_key(|(at, _)| *at);
    let mut rows: Vec<_> = candidates
        .into_iter()
        .map(|(_, r)| *r)
        .filter(|r| r.get("profile_id") == h.get("profile_id") && ids.contains(&pystr(&r["id"])))
        .map(without_provenance)
        .collect();
    rows.sort_by_key(order);
    let enriched = evidence::enriched_rows(&rows)?;
    let canonical = evidence::canonical_input(&enriched, &json!({}), &BTreeSet::new())?;
    let rejected: BTreeSet<_> = array(&canonical["rejected_events"])
        .iter()
        .flat_map(|i| array(&i["transaction_ids"]).iter().map(pystr))
        .collect();
    if !rejected.is_empty() {
        return Err(format!(
            "component evidence contains a rejected canonical event: {}",
            rejected.into_iter().collect::<Vec<_>>().join(", ")
        )
        .into());
    }
    let aliases: BTreeMap<_, _> = array(&canonical["events"])
        .iter()
        .flat_map(|e| {
            array(&e["observation_aliases"])
                .iter()
                .map(|a| (pystr(&a[0]), pystr(&a[1])))
        })
        .collect();
    let missing: Vec<_> = ids
        .iter()
        .filter(|id| !aliases.contains_key(*id))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "component evidence is unavailable for: {}",
            missing.join(", ")
        )
        .into());
    }
    let relevant: BTreeSet<_> = ids.iter().map(|id| aliases[id].clone()).collect();
    let mut snapshots = BTreeMap::new();
    for e in array(&canonical["events"]) {
        for s in array(&e["evidence_snapshots"]) {
            if relevant.contains(&pystr(&s["quantity_hash"])) {
                snapshots.insert(pystr(&s["detail_hash"]), s.clone());
            }
        }
    }
    Ok(snapshots.into_values().collect())
}
// Python computes detail hashes after the canonical-evidence exception guard.
fn commitment_pairs(snapshots: &[Value]) -> Result<Vec<(String, String)>, Error> {
    let mut result = Vec::new();
    for s in snapshots {
        let mut payload = codec::loads(
            s["payload_json"]
                .as_str()
                .ok_or("custody evidence payload is invalid")?,
        )?;
        let Py::Dict(ref mut fields) = payload else {
            return Err("custody evidence payload is invalid".into());
        };
        fields.retain(|(k,_)|!matches!(k,Key::Str(k) if k.as_str().is_some_and(|k|["fingerprint","occurred_at","confirmed_at","raw_json"].contains(&k))));
        let detail_hash = codec::hash_py(&payload, false).map_err(Error::encoding)?.0;
        result.push((pystr(&s["quantity_hash"]), detail_hash));
    }
    result.sort();
    Ok(result)
}
pub(super) fn evidence_status(
    shared: &Shared,
    h: &Value,
    legs: &[Value],
    commitments: &[Value],
) -> Result<Value, Error> {
    let expected = &h["expected_evidence_count"];
    let count = commitments.len();
    let result = |status: &str, valid: bool| json!({"expected_count":expected,"commitment_count":count,"valid":valid,"usable":valid,"status":status,"reason":status});
    if expected.is_null() {
        return Ok(result("commitment_header_missing", false));
    }
    let n = wide(expected);
    if count as i128 != n
        || commitments
            .iter()
            .enumerate()
            .any(|(i, c)| wide(&c["ordinal"]) != i as i128)
    {
        return Ok(result("commitments_incomplete", false));
    }
    let ids = anchors(legs);
    if n == 0 {
        let mut v = result(
            if ids.is_empty() {
                "matched"
            } else {
                "unexpected_transaction_anchors"
            },
            ids.is_empty(),
        );
        v["current_count"] = json!(0);
        return Ok(v);
    }
    if ids.is_empty() {
        return Ok(result("transaction_anchors_missing", false));
    }
    let current = match current(shared, h, legs) {
        Ok(c) => c,
        Err(e) if e.is_evidence_unavailable() => {
            let mut v = result("current_evidence_unavailable", false);
            v["detail"] = json!(e);
            return Ok(v);
        }
        Err(e) => return Err(e),
    };
    let current = commitment_pairs(&current)?;
    let committed: Vec<_> = commitments
        .iter()
        .map(|c| (pystr(&c["quantity_hash"]), pystr(&c["detail_hash"])))
        .collect();
    let matched = current == committed;
    let mut v = result(
        if matched {
            "matched"
        } else {
            "evidence_mismatch"
        },
        matched,
    );
    v["current_count"] = json!(current.len());
    Ok(v)
}
/// Recovered native observations with the lookups each component makes.
pub(super) struct Native {
    observations: Vec<Value>,
    /// First observation per `text(transaction_id)`, as a forward scan finds it.
    by_transaction: HashMap<String, usize>,
    /// Observation positions, in order, per encoded `event_key`; serde's
    /// encoding is injective, so a group holds exactly the equal event keys.
    by_event: HashMap<String, Vec<usize>>,
}
impl Native {
    pub(super) fn new(observations: Vec<Value>) -> Self {
        let mut by_transaction = HashMap::new();
        let mut by_event: HashMap<String, Vec<usize>> = HashMap::new();
        for (at, o) in observations.iter().enumerate() {
            by_transaction
                .entry(text(&o["transaction_id"]))
                .or_insert(at);
            by_event
                .entry(o["event_key"].to_string())
                .or_default()
                .push(at);
        }
        Self {
            observations,
            by_transaction,
            by_event,
        }
    }
    fn event(&self, key: &Value) -> impl Iterator<Item = &Value> {
        self.by_event
            .get(&key.to_string())
            .into_iter()
            .flatten()
            .map(|&at| &self.observations[at])
    }
}
pub(super) fn native_context(snapshot: &Snapshot) -> Result<Vec<Value>, Error> {
    let profile = pystr(&snapshot.profile["id"]);
    let mut rows: Vec<_> = snapshot
        .transactions
        .iter()
        .filter(|r| r.get("profile_id") == Some(&json!(profile)) && !truth(&r["excluded"]))
        .map(without_provenance)
        .collect();
    rows.sort_by_key(order);
    let rows = evidence::enriched_rows(&rows)?;
    let mut observations: Vec<Value> = Vec::new();
    // Position per encoded transaction id; encoding equality is Value equality.
    let mut positions: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let built = evidence::event_key(&row).and_then(|key| evidence::observation(&row, &key));
        match built {
            Ok(o) => {
                if let Some(&at) = positions.get(&o["transaction_id"].to_string()) {
                    observations[at] = o;
                } else {
                    positions.insert(o["transaction_id"].to_string(), observations.len());
                    observations.push(o);
                }
            }
            Err(e) if e.is_value_or_type() => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(observations)
}
pub(super) fn native_status(
    legs: &[Value],
    allocs: &[Value],
    native: &Native,
) -> Result<Value, Error> {
    let boundaries: Vec<_> = legs
        .iter()
        .filter(|l| {
            (l["role"] == "source" || owned(l))
                && truth(or(&l["anchor_transaction_id"], &l["transaction_id"]))
                && big(&l["amount_msat"]) > BigInt::from(0)
        })
        .collect();
    let mut exact = BTreeSet::new();
    let anchor = |l: &Value| text(or(&l["anchor_transaction_id"], &l["transaction_id"]));
    let observation = |id: String| {
        native
            .by_transaction
            .get(&id)
            .map(|&at| &native.observations[at])
    };
    for a in allocs {
        let Some((s, t)) = by_id(legs, &a["source_leg_id"]).zip(by_id(legs, &a["sink_leg_id"]))
        else {
            continue;
        };
        let Some((so, to)) = observation(anchor(s)).zip(observation(anchor(t))) else {
            continue;
        };
        if so["event_key"] == to["event_key"]
            && so["direction"] == "outbound"
            && to["direction"] == "inbound"
            && s["asset"] == t["asset"]
            && big(&a["source_amount_msat"]) == big(&a["sink_amount_msat"])
        {
            exact.insert(text(&t["id"]));
        }
    }
    let mut supported = 0;
    let mut partial = 0;
    let mut contradicted = 0;
    for l in &boundaries {
        if exact.contains(&text(&l["id"])) {
            supported += 1;
            continue;
        }
        let id = anchor(l);
        let Some(o) = observation(id.clone()) else {
            continue;
        };
        if o["event_key"]["native_namespace"] != "chain"
            || !matches!(o["event_key"]["chain"].as_str(), Some("bitcoin" | "liquid"))
        {
            continue;
        }
        let opposite = if o["direction"] == "outbound" {
            "inbound"
        } else {
            "outbound"
        };
        // Only observations of this event key can satisfy the event equality.
        let compatible: Vec<_> = native
            .event(&o["event_key"])
            .filter(|v| {
                text(&v["transaction_id"]) != id
                    && v["direction"] == opposite
                    && v["event_key"] == o["event_key"]
                    && v["asset"] == o["asset"]
            })
            .collect();
        if compatible.is_empty() {
            continue;
        }
        let unique: BTreeMap<_, _> = compatible
            .into_iter()
            .map(|v| (text(&v["quantity_hash"]), v))
            .collect();
        let recovered:BigInt=unique.values().map(|v|anchors::principal(&json!({"amount":v["amount_msat"],"fee":v["fee_msat"],"direction":v["direction"],"amount_includes_fee":v["amount_includes_fee"]}))).collect::<Result<Vec<_>,_>>()?.into_iter().sum();
        let same: BTreeMap<_, _> = native
            .event(&o["event_key"])
            .filter(|v| {
                v["event_key"] == o["event_key"]
                    && v["direction"] == o["direction"]
                    && v["asset"] == o["asset"]
            })
            .map(|v| (text(&v["quantity_hash"]), v))
            .collect();
        let capacity:BigInt=same.values().map(|v|anchors::principal(&json!({"amount":v["amount_msat"],"fee":v["fee_msat"],"direction":v["direction"],"amount_includes_fee":v["amount_includes_fee"]}))).collect::<Result<Vec<_>,_>>()?.into_iter().sum();
        if recovered > capacity {
            contradicted += 1;
        } else if recovered == capacity {
            supported += 1;
        } else {
            partial += 1;
        }
    }
    let status = if contradicted > 0 {
        "contradicted"
    } else if supported == boundaries.len() && !boundaries.is_empty() {
        "corroborated"
    } else if supported > 0 || partial > 0 {
        "partial"
    } else {
        "unverified"
    };
    Ok(
        json!({"boundary_count":boundaries.len(),"supported_boundary_count":supported,"partially_supported_boundary_count":partial,"contradicted_boundary_count":contradicted,"status":status,"usable":status!="contradicted"}),
    )
}
