use crate::analysis_features::{evaluate, extract};
use kassiber_envelope::AppError;
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub mod history;
pub mod inventory;

type Result<T> = std::result::Result<T, AppError>;

pub struct OwnedOutput {
    pub node_id: String,
    pub txid: String,
    pub vout: u32,
    pub wallet_id: String,
    pub amount_msat: BigInt,
    pub script_key: Option<String>,
    pub branch_role: String,
    pub branch_evidence_level: String,
    pub change_evidence: String,
}

pub struct LinkageEdge {
    pub from_node_id: String,
    pub to_node_id: String,
    pub observer_linkage: bool,
}

pub struct AdversaryView {
    pub tier: String,
    pub label: String,
    pub cluster_ids: BTreeSet<String>,
    pub exposed_cluster_count: Value,
    pub model_assumption_count: usize,
}

pub struct Baseline {
    pub nodes: Vec<OwnedOutput>,
    pub edges: Vec<LinkageEdge>,
    pub adversary_views: Vec<AdversaryView>,
    pub historical_linkage_bounded: bool,
}

pub struct Proposal {
    decoded: Value,
    structural: BTreeSet<String>,
}

fn evidence<'a>(levels: impl IntoIterator<Item = &'a str>) -> &'static str {
    let values: BTreeSet<_> = levels.into_iter().collect();
    if values.contains("unknown") {
        "unknown"
    } else if values.contains("derived") {
        "derived"
    } else {
        "exact"
    }
}

fn components(baseline: &Baseline) -> Result<BTreeMap<&str, String>> {
    fn root(parents: &mut [usize], id: usize) -> usize {
        let mut root = id;
        while parents[root] != root {
            root = parents[root];
        }
        let mut current = id;
        while parents[current] != current {
            let parent = parents[current];
            parents[current] = root;
            current = parent;
        }
        root
    }
    let ids: BTreeSet<_> = baseline.nodes.iter().map(|n| n.node_id.as_str()).collect();
    if ids.len() != baseline.nodes.len() {
        return Err(AppError::new(
            "internal_error",
            "Duplicate privacy linkage output",
        ));
    }
    let positions: BTreeMap<_, _> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut parents: Vec<_> = (0..ids.len()).collect();
    for edge in baseline.edges.iter().filter(|e| e.observer_linkage) {
        let missing = || AppError::new("internal_error", "Unknown privacy linkage output");
        let left = *positions
            .get(edge.from_node_id.as_str())
            .ok_or_else(missing)?;
        let right = *positions
            .get(edge.to_node_id.as_str())
            .ok_or_else(missing)?;
        let left = root(&mut parents, left);
        let right = root(&mut parents, right);
        parents[left.max(right)] = left.min(right);
    }
    let mut groups: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for id in ids {
        groups
            .entry(root(&mut parents, positions[id]))
            .or_default()
            .push(id);
    }
    let mut result = BTreeMap::new();
    for (index, ids) in groups.values().enumerate() {
        for id in ids {
            result.insert(*id, format!("inference_cluster:{}", index + 1));
        }
    }
    Ok(result)
}

impl Proposal {
    pub fn decode(text: &str) -> Result<Self> {
        let decoded = crate::psbt::decode_text_structure(text).map_err(|error| {
            if error.code == "internal_error" {
                error
            } else {
                AppError::new("validation", "Could not decode the PSBT")
                    .with_hint("Provide a valid base64 PSBT v0 or v2 with the required transaction fields.")
                    .with_details(kassiber_envelope::dict([("reason", kassiber_envelope::s("PSBT structure or supplied prevout evidence is invalid"))]))
            }
        })?;
        let tx = json!({"version":decoded["version"],"locktime":decoded["locktime"],
            "vin":decoded["inputs"].as_array().unwrap().iter().map(|i| json!({"txid":i["prev_txid"],"vout":i["vout"],"sequence":i["sequence"]})).collect::<Vec<_>>(),
            "vout":decoded["outputs"].as_array().unwrap().iter().map(|o| json!({"value":o["value_msat"].as_u64().unwrap()/1000,"scriptpubkey":o["script_key"]})).collect::<Vec<_>>()});
        let features = extract(&tx, "psbt_proposal", None, &[]);
        let structural = evaluate(&features, None)
            .into_iter()
            .map(|row| row["code"].as_str().unwrap().to_owned())
            .collect();
        Ok(Self {
            decoded,
            structural,
        })
    }

    pub fn analyze(&self, baseline: &Baseline) -> Result<Value> {
        let decoded = &self.decoded;
        let inputs = decoded["inputs"].as_array().unwrap();
        let outputs = decoded["outputs"].as_array().unwrap();
        let component = components(baseline)?;
        let by_outpoint: BTreeMap<_, _> = baseline
            .nodes
            .iter()
            .map(|n| ((n.txid.as_str(), n.vout), n))
            .collect();
        let known: Vec<_> = inputs
            .iter()
            .filter_map(|i| {
                by_outpoint
                    .get(&(
                        i["prev_txid"].as_str().unwrap(),
                        i["vout"].as_u64().unwrap() as u32,
                    ))
                    .copied()
            })
            .collect();
        let unknown = inputs.len() - known.len();
        let ids: BTreeSet<_> = known.iter().map(|n| n.node_id.as_str()).collect();
        let wallets: BTreeSet<_> = known.iter().map(|n| n.wallet_id.as_str()).collect();
        let clusters: BTreeSet<_> = ids.iter().map(|id| component[*id].as_str()).collect();
        let merge = clusters.len().saturating_sub(1);
        let level = if unknown > 0 || baseline.historical_linkage_bounded {
            "unknown"
        } else {
            "derived"
        };
        let mut owned_outputs = 0;
        let mut receive = 0;
        let mut change = 0;
        let mut unknown_outputs = 0;
        let mut receive_clusters = BTreeSet::new();
        let mut items = Vec::new();
        let mut levels = Vec::new();
        let mut by_script: BTreeMap<&str, Vec<&OwnedOutput>> = BTreeMap::new();
        for node in &baseline.nodes {
            if let Some(script) = node.script_key.as_deref().filter(|v| !v.is_empty()) {
                by_script.entry(script).or_default().push(node);
            }
        }
        for output in outputs {
            let Some(matches) = by_script.get(output["script_key"].as_str().unwrap()) else {
                unknown_outputs += 1;
                continue;
            };
            owned_outputs += 1;
            let item_level = evidence(matches.iter().map(|n| n.branch_evidence_level.as_str()));
            levels.push(item_level);
            let (kind, item_level, change_evidence) =
                if matches.iter().any(|n| n.branch_role == "receive") {
                    receive += 1;
                    receive_clusters.extend(
                        matches
                            .iter()
                            .map(|n| component[n.node_id.as_str()].as_str()),
                    );
                    (
                        "existing_receive_output_reuse",
                        item_level,
                        "receive_branch_match",
                    )
                } else if matches.iter().any(|n| n.branch_role == "change") {
                    change += 1;
                    let change_level = ["ground_truth", "imported", "heuristic"]
                        .into_iter()
                        .find(|v| matches.iter().any(|n| n.change_evidence == *v))
                        .unwrap_or("unavailable");
                    ("existing_change_output_match", item_level, change_level)
                } else {
                    levels.push("unknown");
                    ("owned_output_role_unknown", "unknown", "unavailable")
                };
            items.push(json!({"kind":kind,"evidence_level":item_level,"matched_local_output_count":matches.len(),"change_evidence":change_evidence}));
        }
        let reuse_delta = receive_clusters.difference(&clusters).count();
        let change_level = if levels.is_empty() {
            "unknown"
        } else {
            evidence(levels)
        };
        let signature = decoded["signature_material_present"].as_bool().unwrap();
        let clean = decoded["unsigned_tx_clean"].as_bool().unwrap();
        let mut limitations = vec![
            json!({"code":"local_only_no_probe","message":"No network request was made; PSBT analysis used local inventory and the unsigned transaction only.","evidence_level":"exact"}),
            json!({"code":"redacted_psbt_payload","message":"Raw PSBT bytes, addresses, scripts, descriptors, xpubs, backend configuration, raw_json, branch/index values, and derivation paths are omitted.","evidence_level":"exact"}),
            json!({"code":"advisory_no_input_proposal","message":"This analysis is read-only and does not propose transaction inputs or outputs.","evidence_level":"exact"}),
        ];
        if baseline.historical_linkage_bounded {
            limitations.push(json!({"code":"historical_linkage_bounded","message":"The historical observer model reached an analysis bound; additional existing links can change the proposed merge delta.","evidence_level":"unknown"}));
        }
        if unknown > 0 {
            limitations.push(json!({"code":"psbt_inputs_not_in_local_inventory","message":"Some PSBT inputs do not match local watch-only Bitcoin UTXO inventory, so ownership and linkage coverage is incomplete.","evidence_level":"unknown","evidence":{"input_count":unknown}}));
        }
        if signature {
            limitations.push(json!({"code":"psbt_signature_material_present","message":"The PSBT input maps contain signing or finalization material; analysis still reads only transaction graph fields.","evidence_level":"exact"}));
        }
        if !clean {
            limitations.push(json!({"code":"unsigned_transaction_not_clean","message":"The PSBT unsigned transaction carried script or witness bytes; graph analysis continued from decoded vin/vout only.","evidence_level":"unknown"}));
        }
        let mut tells = Vec::new();
        let attribution = if known.is_empty() {
            "unknown"
        } else {
            "emitted_by_you"
        };
        let mut tell = |kind: &str,
                        source: &str,
                        evidence_level: &str,
                        penalizes: bool,
                        evidence: Value| {
            tells.push(json!({"tell_id":format!("{kind}:psbt:{}",tells.len()),"kind":kind,"attribution":attribution,"evidence_level":evidence_level,"source":source,"penalizes_wallet":penalizes,"evidence":evidence}));
        };
        if inputs.len() > 1 {
            tell(
                "sender_common_input",
                "decoded_psbt_unsigned_tx",
                if unknown > 0 { "unknown" } else { "derived" },
                !known.is_empty(),
                json!({"input_count":inputs.len(),"known_owned_input_count":known.len(),"unknown_input_count":unknown}),
            );
        }
        if self.structural.contains("explicit_rbf_signal") {
            tell(
                "sender_rbf",
                "decoded_psbt_unsigned_tx",
                "exact",
                !known.is_empty(),
                json!({"rbf_signaled":true}),
            );
        }
        if self.structural.contains("op_return_output") {
            tell(
                "op_return_output",
                "decoded_psbt_unsigned_tx",
                "exact",
                !known.is_empty(),
                json!({"op_return_present":true}),
            );
        }
        let input_value: BigInt = known.iter().map(|n| &n.amount_msat).sum();
        let output_value: BigInt = outputs
            .iter()
            .map(|o| BigInt::from(o["value_msat"].as_u64().unwrap()))
            .sum();
        let fee_known = !inputs.is_empty()
            && unknown == 0
            && known.len() == inputs.len()
            && input_value >= output_value;
        tell(
            "fee_observation",
            "decoded_psbt_amounts",
            if fee_known { "exact" } else { "unknown" },
            false,
            json!({"fee_known":fee_known,"fee_value_redacted":true,"unknown_input_count":unknown}),
        );
        let mut findings = Vec::new();
        let mut finding = |id: &str,
                           kind: &str,
                           severity: &str,
                           title: &str,
                           detail: String,
                           evidence_level: &str,
                           evidence: Value| {
            findings.push(json!({"id":id,"kind":kind,"severity":severity,"title":title,"detail":detail,"evidence_level":evidence_level,"evidence":evidence}));
        };
        if merge > 0 {
            finding("psbt_cluster_merge","cluster_merge","warning","PSBT may link local clusters under a common-input assumption",format!("Assuming an ordinary non-collaborative spend, the decoded transaction would create {merge} common-input cluster merge(s) among locally known inputs."),level,json!({"cluster_merge_delta":merge}));
        }
        if unknown > 0 {
            finding("psbt_unknown_inputs","unknown_inputs","warning","PSBT has inputs outside local inventory",format!("{unknown} input(s) do not match local watch-only Bitcoin UTXO inventory, so the graph delta is incomplete."),"unknown",json!({"unknown_input_count":unknown}));
        }
        if receive > 0 {
            finding("psbt_existing_receive_reuse","receive_reuse",if reuse_delta > 0 { "warning" } else { "info" },"PSBT output matches an existing local receive script",format!("{receive} output(s) match existing local receive evidence; the reduced payload omits script data."),"exact",json!({"receive_reuse_output_count":receive,"receive_reuse_cluster_delta":reuse_delta}));
        }
        if signature {
            finding("psbt_signature_material_present","signature_material_present","info","PSBT input maps contain signing material","The parser detected input-map signing or finalization material; the privacy analysis used only graph fields.".into(),"exact",json!({"signature_material_present":true}));
        }
        if !clean {
            finding("psbt_unsigned_tx_not_clean","unsigned_tx_not_clean","warning","Unsigned transaction contains script or witness bytes","The decoded unsigned transaction was not empty-script clean; the graph delta may be incomplete.".into(),"unknown",json!({"unsigned_tx_clean":false}));
        }
        let adversary: Vec<_> = baseline.adversary_views.iter().map(|view| {
            let exposed: BTreeSet<_> = view.cluster_ids.iter().map(String::as_str).collect();
            let visible = view.tier == "passive_chain_watcher" || !clusters.is_disjoint(&exposed);
            json!({"tier":view.tier,"label":view.label,"evidence_level":if unknown > 0 { "unknown" } else { "derived" },"source":"psbt_common_input_overlay","cluster_merge_delta":if visible { merge } else { 0 },"owned_input_cluster_count":clusters.len(),"newly_exposed_component_count":if visible { clusters.difference(&exposed).count() } else { 0 },"evidence":{"current_exposed_cluster_count":view.exposed_cluster_count,"unknown_input_count":unknown,"model_assumption_count":view.model_assumption_count}})
        }).collect();
        let supported = "supported_by_local_ground_truth";
        let unsupported = "not_supported_by_local_ground_truth_under_this_model";
        let what_if = vec![
            json!({"scenario":"fresh_receive_output","evidence_level":"derived","support_status":supported,"cluster_merge_delta":0,"evidence":{"modeled_output_reuse":false,"assumption":"A fresh receive output is modeled as absent from the current local graph."}}),
            json!({"scenario":"existing_receive_reuse","evidence_level":if receive > 0 { "exact" } else { "unknown" },"support_status":if receive > 0 { supported } else { unsupported },"cluster_merge_delta":reuse_delta,"evidence":{"matched_existing_receive_output_count":receive,"receive_reuse_cluster_delta":reuse_delta}}),
            json!({"scenario":"hypothetical_input_consolidation","evidence_level":level,"support_status":if clusters.len() > 1 { supported } else { unsupported },"cluster_merge_delta":merge,"evidence":{"owned_input_cluster_count":clusters.len(),"cluster_merge_delta":merge}}),
        ];
        Ok(
            json!({"payload_schema_version":1,"redaction":"ai_export_safe","local_only":true,"read_only":true,"advisory_only":true,"signing_supported":false,"broadcast_supported":false,"psbt_exposure":"reduced_findings_only",
            "summary":{"evidence_level":level,"decode_status":"decoded","input_count":inputs.len(),"output_count":outputs.len(),"known_owned_input_count":known.len(),"unknown_input_count":unknown,"owned_input_cluster_count":clusters.len(),"cluster_merge_delta":merge,"receive_reuse_output_count":receive,"receive_reuse_cluster_delta":reuse_delta,"blast_radius_score":merge+reuse_delta,"transaction_tell_count":tells.len(),"adversary_delta_count":adversary.len(),"finding_count":findings.len(),"limitation_count":limitations.len(),"signature_material_present":signature},
            "cluster_merge":{"kind":"psbt_common_input_overlay","evidence_level":level,"owned_input_count":known.len(),"owned_input_node_count":ids.len(),"owned_input_cluster_count":clusters.len(),"owned_input_wallet_count":wallets.len(),"cluster_merge_delta":merge,"affected_cluster_ids":clusters,"affected_node_ids":ids,"unknown_input_count":unknown},
            "change_evidence":{"evidence_level":change_level,"owned_output_match_count":owned_outputs,"receive_reuse_output_count":receive,"change_like_output_count":change,"unknown_output_count":unknown_outputs,"receive_reuse_cluster_delta":reuse_delta,"items":items},
            "unknowns":{"evidence_level":if unknown > 0 || baseline.historical_linkage_bounded { "unknown" } else { "exact" },"input_count":unknown,"output_without_local_match_count":unknown_outputs,"coverage_complete":unknown == 0 && !baseline.historical_linkage_bounded},
            "transaction_tells":tells,"adversary_deltas":adversary,"what_if":what_if,"findings":findings,"limitations":limitations}),
        )
    }
}

#[cfg(any(test, feature = "test-harness"))]
pub fn fixture_baseline(value: &Value) -> Baseline {
    Baseline {
        nodes: value["nodes"]
            .as_object()
            .unwrap()
            .values()
            .map(|n| OwnedOutput {
                node_id: n["node_id"].as_str().unwrap().into(),
                txid: n["txid"].as_str().unwrap().into(),
                vout: n["vout"].as_u64().unwrap().try_into().unwrap(),
                wallet_id: n["wallet_id"].as_str().unwrap().into(),
                amount_msat: n["amount_msat"].to_string().parse().unwrap(),
                script_key: n["script_key"].as_str().map(str::to_owned),
                branch_role: n["branch_role"].as_str().unwrap().into(),
                branch_evidence_level: n["branch_evidence_level"].as_str().unwrap().into(),
                change_evidence: n["change_evidence"].as_str().unwrap().into(),
            })
            .collect(),
        edges: value["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| LinkageEdge {
                from_node_id: e["from_node_id"].as_str().unwrap().into(),
                to_node_id: e["to_node_id"].as_str().unwrap().into(),
                observer_linkage: e["observer_linkage"].as_bool().unwrap(),
            })
            .collect(),
        adversary_views: value["adversary_views"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| AdversaryView {
                tier: v["tier"].as_str().unwrap().into(),
                label: v["label"].as_str().unwrap().into(),
                cluster_ids: v["clusters"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| c["cluster_id"].as_str().unwrap().into())
                    .collect(),
                exposed_cluster_count: v["summary"]
                    .get("exposed_cluster_count")
                    .cloned()
                    .unwrap_or(json!(0)),
                model_assumption_count: v["model_assumptions"].as_array().unwrap().len(),
            })
            .collect(),
        historical_linkage_bounded: value["limitations"].as_array().unwrap().iter().any(|v| {
            v["code"] == "shared_analysis_bounded"
                && (v["evidence"]["budget_exhausted"] == true
                    || v["evidence"]["stopped_reasons"]
                        .as_array()
                        .is_some_and(|r| r.iter().any(|v| v == "hypothesis_budget")))
        }),
    }
}

#[cfg(feature = "test-harness")]
pub fn fixture_no_egress<T>(run: impl FnOnce() -> T) -> T {
    let gate = kassiber_net::Gate::new(
        || kassiber_net::Offline::Unavailable,
        kassiber_net::Ledger::default(),
    );
    let result = run();
    assert!(gate.attempts().records.is_empty());
    result
}
