//! Consolidation -> recorded fan-out -> ownership. This is still a partial
//! interpreter: holds, channels, HTLC production and reconciliation are pending.
use super::{anchor, array, consolidation, fanout, ownership, pairs, recorded, samourai, Context};
use crate::{
    codec::{self, truth},
    snapshot::Snapshot,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default, Serialize, serde::Deserialize)]
pub(super) struct Profile {
    consolidation: ownership::Derivation,
    ownership: ownership::Derivation,
    fanout: ownership::Derivation,
}

fn rows_after(rows: &[Value], result: &ownership::Derivation) -> Vec<Value> {
    let mut next: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let id = codec::pystr(&row["id"]);
            if result.dropped_out_ids.contains(&id) || result.dropped_in_ids.contains(&id) {
                return None;
            }
            Some(result.out_row_overrides.get(&id).unwrap_or(row).clone())
        })
        .collect();
    next.extend(result.synthetic_rows.iter().cloned());
    // The journal calls derive_profile_transfers with sort_key=None.
    next
}

pub(super) fn derive(
    rows: &[Value],
    snapshot: &Snapshot,
    initial: &BTreeSet<String>,
) -> Result<Profile, String> {
    let mut handled = initial.clone();
    let consolidation = consolidation::derive(rows, snapshot, &handled)?;
    let rows = rows_after(rows, &consolidation);
    handled.extend(
        consolidation
            .dropped_out_ids
            .iter()
            .chain(&consolidation.dropped_in_ids)
            .cloned(),
    );
    let fanout: ownership::Derivation = serde_json::from_value(fanout::derive(&rows, &handled)?)
        .expect("internal fan-out derivation");
    let rows = rows_after(&rows, &fanout);
    handled.extend(
        fanout
            .dropped_out_ids
            .iter()
            .chain(&fanout.dropped_in_ids)
            .cloned(),
    );
    handled.extend(fanout.out_row_overrides.keys().cloned());
    handled.extend(
        fanout
            .derived_pairs
            .iter()
            .map(|pair| codec::pystr(&pair["in"]["id"])),
    );
    let ownership = ownership::derive(&rows, snapshot, &handled)?;
    Ok(Profile {
        consolidation,
        ownership,
        fanout,
    })
}

fn annotate(profile: &Profile) -> Vec<Value> {
    let mut result = profile.consolidation.derived_pairs.clone();
    for pair in &profile.ownership.derived_pairs {
        let mut pair = pair.clone();
        let source = anchor(&pair["out"]);
        if profile.ownership.out_row_overrides.contains_key(&source) {
            pair.as_object_mut()
                .unwrap()
                .entry("group_id")
                .or_insert_with(|| json!(format!("ownership-event:{source}")));
            pair.as_object_mut()
                .unwrap()
                .entry("allow_unclaimed_residual")
                .or_insert(json!(true));
        }
        result.push(pair);
    }
    result.extend(profile.fanout.derived_pairs.iter().cloned());
    result
}

fn quarantines(profile: &Profile, rows: &[Value], context: &Context) -> Result<Vec<Value>, String> {
    let mut members = BTreeMap::new();
    for event in array(&context.canonical["events"]) {
        let mut ids: Vec<_> = array(&event["observation_aliases"])
            .iter()
            .map(|alias| codec::text(&alias[0]))
            .collect();
        ids.sort();
        for id in &ids {
            members.insert(id.clone(), ids.clone());
        }
    }
    let by_id: BTreeMap<_, _> = rows
        .iter()
        .map(|row| {
            (
                if truth(&row["id"]) {
                    codec::pystr(&row["id"])
                } else {
                    String::new()
                },
                row,
            )
        })
        .collect();
    let mut result = BTreeMap::new();
    for blocked in profile
        .consolidation
        .blocked_sources
        .iter()
        .chain(&profile.ownership.blocked_sources)
        .chain(&profile.fanout.blocked_sources)
    {
        let row = &blocked["row"];
        let id = if truth(&row["id"]) {
            codec::pystr(&row["id"])
        } else {
            String::new()
        };
        let reason = if truth(&blocked["reason"]) {
            codec::pystr(&blocked["reason"])
        } else {
            "ownership_transfer_unresolved".into()
        };
        let ids = members.get(&id).cloned().unwrap_or_else(|| vec![id]);
        let mut detail = blocked["detail"].as_object().cloned().unwrap_or_default();
        detail
            .entry("atomic_event_transaction_ids")
            .or_insert_with(|| json!(ids));
        for id in ids {
            if context.excluded.contains(&id) {
                continue;
            }
            let row = by_id.get(&id).copied().unwrap_or(row);
            result.insert((id.clone(), reason.clone()),json!({"transaction_id":id,"workspace_id":row["workspace_id"],"profile_id":row["profile_id"],"reason":reason,"detail_json":pairs::detail_json(&Value::Object(detail.clone()))?}));
        }
    }
    Ok(result.into_values().collect())
}

pub(super) fn compile(snapshot: &Snapshot, context: &Context) -> Result<Value, String> {
    let run = || {
        let rows: Vec<_> = snapshot.transactions.iter().map(|r| json!(r)).collect();
        let recorded = recorded::detect(&rows)?.0;
        let mut handled = recorded::resolved_ids(&recorded, context, true);
        handled.extend(
            array(&samourai::compile(snapshot, context)?["touched_ids"])
                .iter()
                .map(codec::text),
        );
        let eligible = snapshot.owned_index.is_some()
            || array(&context.canonical["events"])
                .iter()
                .any(|event| event["event_key"]["native_namespace"] == "chain");
        let mut normalized = rows.clone();
        if eligible {
            for row in &mut normalized {
                if let Some(observation) = context.observations.get(&anchor(row)) {
                    if row["observation_fee_attribution"] == "implicit_wallet_delta"
                        && observation["fee_attribution"] == "exact"
                    {
                        row["amount"] = observation["amount_msat"].clone();
                        row["fee"] = observation["fee_msat"].clone();
                        row["observation_fee_attribution"] = json!("exact");
                        row["custody_native_fee_normalized"] = json!(true);
                    }
                }
            }
        }
        let profile = if eligible {
            derive(&normalized, snapshot, &handled)?
        } else {
            Profile::default()
        };
        let candidates = annotate(&profile);
        let compiled = pairs::compile(&candidates, context, snapshot)?;
        Ok::<_, String>(
            json!({"derivation":profile,"handled_ids":handled,"derivation_rows":normalized,"rows_after_consolidation":rows_after(&normalized,&profile.consolidation),"rows_after_fanout":rows_after(&rows_after(&normalized,&profile.consolidation),&profile.fanout),"derived_pairs":candidates,"claims":compiled["claims"],"blocked_transaction_ids":compiled["blocked_transaction_ids"],"quarantines":compiled["quarantines"],"derivation_quarantines":quarantines(&profile,&rows,context)?,"native_audits":pairs::native_audits(&candidates,context)?}),
        )
    };
    Ok(run().unwrap_or_else(|error| json!({"error":error})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipeline_helper_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_pipeline_helpers.json"
        ))
        .unwrap();
        for vector in vectors {
            let actual = match vector["op"].as_str().unwrap() {
                "rows" => json!(rows_after(
                    array(&vector["rows"]),
                    &serde_json::from_value(vector["derivation"].clone()).unwrap()
                )),
                "annotate" => json!(annotate(
                    &serde_json::from_value(vector["profile"].clone()).unwrap()
                )),
                "quarantine" => {
                    let context = Context {
                        observations: BTreeMap::new(),
                        canonical: vector["canonical"].clone(),
                        excluded: array(&vector["excluded"]).iter().map(codec::text).collect(),
                    };
                    json!(quarantines(
                        &serde_json::from_value(vector["profile"].clone()).unwrap(),
                        array(&vector["rows"]),
                        &context
                    )
                    .unwrap())
                }
                _ => unreachable!(),
            };
            assert_eq!(actual, vector["expected"], "{}", vector["case"]);
        }
    }
    #[test]
    fn pipeline_production_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_pipeline_cases.json"
        ))
        .unwrap();
        for vector in vectors {
            let snapshot: Snapshot = serde_json::from_value(vector["input"].clone()).unwrap();
            let handled = array(&vector["handled_ids"])
                .iter()
                .map(codec::text)
                .collect();
            let result = derive(array(&vector["rows"]), &snapshot, &handled);
            if vector.get("error").is_some() {
                assert_eq!(
                    result.err().map(|e| json!(e)),
                    Some(vector["error"].clone()),
                    "{}",
                    vector["case"]
                );
            } else {
                assert_eq!(
                    json!(result.unwrap()),
                    vector["expected"],
                    "{}",
                    vector["case"]
                );
            }
        }
    }
}
