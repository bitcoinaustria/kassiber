use super::value::{json, Value};
use super::{
    confirmations::ConfirmationOverlay,
    occurrences::{OccurrenceResolver, SourceIdentity},
    AnalysisIndex,
};
use kassiber_envelope::AppError;
use kassiber_pyjson::{BigInt, PyStr};
use kassiber_store::rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};

mod normalize;
pub(super) use normalize::scope;
use normalize::*;

type Members = (BTreeSet<String>, BTreeSet<String>);

pub(super) struct ObservationBuilder<'a> {
    conn: &'a Connection,
    profile: String,
    occurrences: OccurrenceResolver<'a>,
    pub(super) nodes: BTreeMap<String, Value>,
    pub(super) edges: BTreeMap<String, Value>,
    edge_order: Vec<String>,
    orders: BTreeMap<&'static str, Vec<String>>,
    subjects: BTreeMap<PyStr, BTreeSet<String>>,
    subject_order: Vec<PyStr>,
    tx_facts: BTreeMap<String, Value>,
    pub(super) output_facts: BTreeMap<String, Value>,
    findings: Vec<Value>,
    pub(super) coverage: Value,
    ranks: BTreeMap<(String, String), u32>,
    shapes: BTreeMap<String, BTreeSet<String>>,
    complete_members: BTreeMap<String, Members>,
    pub(super) records: BTreeMap<String, String>,
    seeds: BTreeSet<String>,
    pub(super) labels: Vec<Value>,
}

fn reference_value(source: &str, id: &str, level: &str) -> Value {
    json!({"source":source,"reference":id,"level":level})
}
fn add_unique(list: &mut Value, value: Value) {
    let list = list.as_array_mut().unwrap();
    if !list.contains(&value) {
        list.push(value);
    }
}
fn fact() -> Value {
    json!({"inputs":[],"outputs":[],"fee_msat":null,"complete":false,"collaboration":null,"transaction_ids":[],"directions":[]})
}
fn point(item: &Value) -> Option<(String, BigInt)> {
    let id = canonical(&item["txid"])?;
    if id == "0".repeat(64) {
        return None;
    }
    let index = crate::scan::integer(&item["vout"].serde())?;
    (index >= BigInt::from(0)).then_some((id, index))
}

impl<'a> ObservationBuilder<'a> {
    pub(super) fn restore_node(&mut self, id: &str, value: Value) {
        if !self.nodes.contains_key(id) {
            self.orders.entry("nodes").or_default().push(id.to_owned());
        }
        self.nodes.insert(id.to_owned(), value);
    }
    pub(super) fn restore_output(&mut self, id: &str, value: Value) {
        if !self.output_facts.contains_key(id) {
            self.orders
                .entry("output_facts")
                .or_default()
                .push(id.to_owned());
        }
        self.output_facts.insert(id.to_owned(), value);
    }
    pub fn read(conn: &'a Connection, profile: &str) -> Result<Self, AppError> {
        Ok(Self {
            conn,
            profile: profile.to_owned(),
            occurrences: OccurrenceResolver::read(conn, profile)?,
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            edge_order: Vec::new(),
            orders: BTreeMap::new(),
            subjects: BTreeMap::new(),
            subject_order: Vec::new(),
            tx_facts: BTreeMap::new(),
            output_facts: BTreeMap::new(),
            findings: Vec::new(),
            coverage: json!({"local_only":true,"egress":"none","missing_tables":[],"invalid_observations":0,"cache_rejected":0,"source_rows":{},"custody_fresh":false}),
            ranks: BTreeMap::new(),
            shapes: BTreeMap::new(),
            complete_members: BTreeMap::new(),
            records: BTreeMap::new(),
            seeds: BTreeSet::new(),
            labels: Vec::new(),
        })
    }
    fn invalid(&mut self, cached: bool) {
        let name = if cached {
            "cache_rejected"
        } else {
            "invalid_observations"
        };
        self.coverage[name] = json!(self.coverage[name].as_u64().unwrap() + 1);
    }
    pub(super) fn finding(
        &mut self,
        code: &str,
        ids: Vec<String>,
        detail: &str,
        info: bool,
        refs: Vec<Value>,
    ) -> Result<(), AppError> {
        let mut ids = ids;
        ids.sort();
        let words = code.replace('_', " ");
        let title = words[..1].to_uppercase() + &words[1..];
        self.findings.push(json!({"id":digest(&json!([code,ids,detail]))?,"code":code,"severity":if info {"info"} else {"warning"},"title":title,"detail":detail,"node_ids":ids,"edge_ids":[],"evidence":refs}));
        Ok(())
    }
    pub(super) fn alias(&mut self, alias: &Value, id: &str) {
        if !alias.is_null() && alias != "" {
            let alias = string(alias);
            if !self.subjects.contains_key(&alias) {
                self.subject_order.push(alias.clone());
            }
            self.subjects
                .entry(alias)
                .or_default()
                .insert(id.to_owned());
        }
    }
    fn node(
        &mut self,
        id: &str,
        chain: Option<&str>,
        network: Option<&str>,
        kind: &str,
        txid: Option<&str>,
    ) {
        if self.nodes.contains_key(id) {
            return;
        }
        let mut node = json!({"id":id,"kind":kind,"chain":chain,"network":network,"label":txid.map(|v| &v[..12]).unwrap_or(kind),"wallet_ids":[],"amount_msat":null,"asset":if chain==Some("bitcoin") {Some("BTC")} else {None},"status":"missing","evidence":[]});
        if let Some(txid) = txid {
            node["txid"] = json!(txid);
        }
        self.nodes.insert(id.to_owned(), node);
        self.orders.entry("nodes").or_default().push(id.to_owned());
        self.alias(&json!(id), id);
    }
    fn merge(
        &mut self,
        id: &str,
        updates: &[(&str, Value)],
        rank: u32,
        evidence: &Value,
    ) -> Result<(), AppError> {
        add_unique(
            &mut self.nodes.get_mut(id).unwrap()["evidence"],
            evidence.clone(),
        );
        for (field, value) in updates {
            if value.is_null() {
                continue;
            }
            let key = (id.to_owned(), (*field).to_owned());
            let previous_rank = self.ranks.get(&key).copied();
            let previous = &self.nodes[id][*field];
            if *field == "confirmations"
                && previous_rank == Some(rank)
                && !previous.is_null()
                && previous != value
            {
                self.nodes.get_mut(id).unwrap()[*field] = Value::Null;
                self.ranks.insert(key, 100);
            } else if previous_rank == Some(rank)
                && !previous.is_null()
                && previous != value
                && ["amount_msat", "asset", "address"].contains(field)
            {
                self.nodes.get_mut(id).unwrap()["status"] = json!("conflicting");
                self.finding("conflicting_output_observations", vec![id.to_owned()], &format!("Retained local observations disagree about {field}; traversal stops at this output."),false,vec![evidence.clone()])?;
                self.nodes.get_mut(id).unwrap()[*field] = Value::Null;
                self.ranks.insert(key, 100);
            } else if previous_rank.is_none_or(|v| rank > v)
                || previous.is_null() && previous_rank.is_none_or(|v| v < 100)
            {
                self.nodes.get_mut(id).unwrap()[*field] = value.clone();
                self.ranks.insert(key, rank);
            }
        }
        Ok(())
    }
    fn own(&mut self, id: &str, wallet: &Value) {
        if truth(wallet) && !array(&self.nodes[id]["wallet_ids"]).contains(wallet) {
            add_unique(
                &mut self.nodes.get_mut(id).unwrap()["wallet_ids"],
                wallet.clone(),
            );
            self.alias(wallet, id);
            self.alias(&json!(format!("wallet:{}", text(wallet))), id);
        }
    }
    fn edge(
        &mut self,
        source: &str,
        target: &str,
        kind: &str,
        evidence: &Value,
        suffix: &str,
        level: &str,
    ) -> Result<(String, bool), AppError> {
        let id = format!(
            "{kind}:{}",
            &digest(&json!([source, target, suffix]))?[..32]
        );
        let created = !self.edges.contains_key(&id);
        if created {
            self.edges.insert(id.clone(), json!({"id":id,"source":source,"target":target,"kind":kind,"evidence_level":level,"amount_msat":null,"asset":null,"label":kind,"status":"observed","evidence":[]}));
            self.edge_order.push(id.clone());
        }
        add_unique(
            &mut self.edges.get_mut(&id).unwrap()["evidence"],
            evidence.clone(),
        );
        Ok((id, created))
    }
    #[allow(clippy::too_many_arguments)]
    fn output(
        &mut self,
        chain: &str,
        network: &str,
        txid: &str,
        vout: &BigInt,
        entry: &Value,
        evidence: &Value,
        rank: u32,
        source: &SourceIdentity,
        occurrence: &Value,
        resolve: bool,
    ) -> Result<String, AppError> {
        let parent = self.occurrences.transaction_id(
            chain,
            network,
            txid,
            source,
            &py(occurrence),
            resolve,
        )?;
        self.node(
            &parent,
            Some(chain),
            Some(network),
            "transaction",
            Some(txid),
        );
        self.alias(&json!(txid), &parent);
        let id = format!(
            "{}:out:{txid}:{vout}",
            parent.rsplit_once(":tx:").unwrap().0
        );
        self.node(&id, Some(chain), Some(network), "output", Some(txid));
        self.merge(
            &id,
            &[(
                "confirmations",
                self.nodes[&parent]["confirmations"].clone(),
            )],
            rank,
            evidence,
        )?;
        let outpoint = format!("{txid}:{vout}");
        self.nodes.get_mut(&id).unwrap()["outpoint"] = json!(outpoint);
        self.nodes.get_mut(&id).unwrap()["label"] = json!(format!("{}:{vout}", &txid[..10]));
        let amount = crate::analysis_features::value_sats(&entry.serde())
            .filter(|n| n >= &BigInt::from(0))
            .map(|n| (n * BigInt::from(1000)).to_string());
        let raw_asset = if truth(&entry["asset_id"]) {
            &entry["asset_id"]
        } else {
            &entry["asset"]
        };
        let asset = if chain == "bitcoin" {
            json!("BTC")
        } else if truth(raw_asset) {
            json!(string(raw_asset))
        } else {
            Value::Null
        };
        self.merge(
            &id,
            &[
                ("amount_msat", json!(amount)),
                ("asset", asset.clone()),
                ("address", address(entry)),
            ],
            rank,
            evidence,
        )?;
        if self.nodes[&id]["status"] == "missing" && truth(entry) {
            self.nodes.get_mut(&id).unwrap()["status"] =
                json!(if rank == 1 { "reference" } else { "observed" });
        }
        if !self.output_facts.contains_key(&id) {
            self.orders
                .entry("output_facts")
                .or_default()
                .push(id.clone());
        }
        let facts = self
            .output_facts
            .entry(id.clone())
            .or_insert_with(|| json!({"script":null,"branch_role":null,"ownership_known":false}));
        if let Some(script) = script(entry).filter(|v| !v.is_empty()) {
            let key = (id.clone(), "script".to_owned());
            let previous = self.ranks.get(&key).copied();
            if previous == Some(rank) && !facts["script"].is_null() && facts["script"] != script {
                self.nodes.get_mut(&id).unwrap()["status"] = json!("conflicting");
                facts["script"] = Value::Null;
                self.ranks.insert(key, 100);
                self.finding("conflicting_output_script",vec![id.clone()],"Retained observations disagree about the locking script; reuse and control hypotheses are unavailable.",false,vec![evidence.clone()])?;
            } else if previous.is_none_or(|v| rank > v) {
                self.output_facts.get_mut(&id).unwrap()["script"] = json!(script);
                self.ranks.insert(key, rank);
            }
            self.alias(&json!(format!("script:{script}")), &id);
        }
        let public_value = chain == "bitcoin"
            || rank < 3
                && entry["role"] != "owned"
                && !truth(&entry["valuecommitment"])
                && !truth(&entry["value_commitment"])
                && (matches!(entry["role"].as_str(), Some("fee" | "external"))
                    || entry.get("value").is_some());
        let public_asset = chain == "bitcoin"
            || public_value
                && !truth(&entry["assetcommitment"])
                && !truth(&entry["asset_commitment"]);
        let facts = self.output_facts.get_mut(&id).unwrap();
        for (key, value) in [
            (
                "public_amount_msat",
                if public_value {
                    json!(amount)
                } else {
                    Value::Null
                },
            ),
            (
                "public_asset",
                if public_asset { asset } else { Value::Null },
            ),
        ] {
            let conflict = format!("{key}_conflict");
            if !value.is_null() && !truth(&facts[&conflict]) {
                if !facts[key].is_null() && facts[key] != value {
                    facts[key] = Value::Null;
                    facts[&conflict] = json!(true);
                } else {
                    facts[key] = value;
                }
            }
        }
        self.alias(&json!(outpoint), &id);
        self.alias(&self.nodes[&id]["address"].clone(), &id);
        self.edge(&parent, &id, "creates", evidence, "", "observed")?;
        Ok(id)
    }

    pub fn transaction(&mut self, row: &Value, cached: bool) -> Result<(), AppError> {
        validate_mappings(row, &["raw_json", "wallet_config_json", "config_json"])?;
        if !self.occurrences.accepts(&py(row), false) {
            self.invalid(false);
            return Ok(());
        }
        let Ok((chain, network)) = scope(row) else {
            self.invalid(false);
            return Ok(());
        };
        let raw = mapping(&row["raw_json"], true);
        let external = canonical(&row["external_id"]);
        let raw_id = canonical(&raw["txid"]);
        if external.is_some() && raw_id.is_some() && external != raw_id {
            self.invalid(false);
            return Ok(());
        }
        let txid = raw_id.or(external);
        let is_chain = ["bitcoin", "liquid"].contains(&chain.as_str()) && txid.is_some();
        let source = self.occurrences.source(&py(row), &chain, &network)?;
        let occurrence = json!(source.occurrence_id);
        let id = if is_chain {
            self.occurrences.transaction_id(
                &chain,
                &network,
                txid.as_ref().unwrap(),
                &source,
                &py(&occurrence),
                true,
            )?
        } else {
            format!("{chain}:{network}:record:{}", text(&row["id"]))
        };
        self.node(
            &id,
            Some(&chain),
            Some(&network),
            if is_chain { "transaction" } else { "record" },
            txid.as_deref().filter(|_| is_chain),
        );
        let evidence = reference_value(
            if cached {
                "reference_cache"
            } else {
                "stored_transaction"
            },
            &text(&row["id"]),
            if cached { "reference" } else { "observed" },
        );
        let rank = if cached { 1 } else { 2 };
        if self.nodes[&id]["status"] == "missing"
            || self.nodes[&id]["status"] == "reference" && !cached
        {
            self.nodes.get_mut(&id).unwrap()["status"] =
                json!(if cached { "reference" } else { "observed" });
        }
        if !cached {
            if self.nodes[&id].get("transaction_id").is_none() {
                self.nodes.get_mut(&id).unwrap()["transaction_id"] = row["id"].clone();
            }
            self.records.insert(text(&row["id"]), id.clone());
            self.alias(&row["id"], &id);
            self.alias(&json!(format!("record:{}", text(&row["id"]))), &id);
            self.own(&id, &row["wallet_id"]);
            self.seeds.insert(id.clone());
        }
        self.merge(
            &id,
            &[("occurred_at", row["occurred_at"].clone())],
            rank,
            &evidence,
        )?;
        if !is_chain {
            self.merge(
                &id,
                &[
                    (
                        "amount_msat",
                        json!(integer(&row["amount"]).map(|v| v.to_string())),
                    ),
                    ("asset", row["asset"].clone()),
                ],
                rank,
                &evidence,
            )?;
            return Ok(());
        }
        let txid = txid.unwrap();
        self.alias(&json!(txid), &id);
        if nonnegative(&raw["confirmations"]) {
            self.merge(
                &id,
                &[("confirmations", raw["confirmations"].clone())],
                rank,
                &evidence,
            )?;
        } else if raw["status"]["confirmed"] == false {
            self.merge(&id, &[("confirmations", json!(0))], rank, &evidence)?;
        }
        if matches!(&raw["confirmations"], Value::Int(n) if n < &BigInt::from(0))
            || raw["removed"] == true
        {
            self.nodes.get_mut(&id).unwrap()["status"] = json!("stale");
            self.finding("retracted_transaction",vec![id.clone()],"Stored transaction is removed or conflicted; it cannot prove current reachability.",false,vec![])?;
        }
        if !self.tx_facts.contains_key(&id) {
            self.orders
                .entry("transaction_facts")
                .or_default()
                .push(id.clone());
        }
        let mut facts = self.tx_facts.get(&id).cloned().unwrap_or_else(fact);
        if !cached {
            add_unique(&mut facts["transaction_ids"], row["id"].clone());
            let direction =
                kassiber_pytext::lower(kassiber_pytext::strip(&or_text(&row["direction"])));
            if !direction.is_empty() {
                add_unique(&mut facts["directions"], json!(direction));
            }
        }
        if chain == "bitcoin" {
            let source = if cached {
                "reference_cache"
            } else {
                "stored_transaction"
            };
            let features = crate::analysis_features::normalize(
                &raw[crate::analysis_features::PERSISTED_FEATURE_KEY].serde(),
                source,
                Some(&id),
            )
            .unwrap_or_else(|| {
                crate::analysis_features::extract(&raw.serde(), source, Some(&id), &[])
            });
            let mut features = json!(features);
            if let Some(previous) = facts["features"]["features"].as_array() {
                for item in features["features"].as_array_mut().unwrap() {
                    if let Some(prior) = previous
                        .iter()
                        .find(|v| v["code"] == item["code"])
                        .filter(|v| v["availability"] != "unavailable")
                    {
                        if item["availability"] == "unavailable" || prior["value"] == item["value"]
                        {
                            *item = prior.clone();
                        } else if prior["availability"] == "conflicting"
                            || item["availability"] == "observed"
                                && prior["availability"] == "observed"
                        {
                            item["value"] = Value::Null;
                            item["availability"] = json!("conflicting");
                            item["confidence"] = json!("unavailable");
                        }
                    }
                }
            }
            facts["features"] = features;
        }
        facts["collaboration"] = stronger(
            facts["collaboration"].clone(),
            collaboration(row, &mapping(&row["raw_json"], false)),
        );
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let vin = array(&raw["vin"]);
        let mut valid_inputs = !vin.is_empty();
        let mut coinbase = false;
        for item in vin {
            if !item.is_object() {
                valid_inputs = false;
                continue;
            }
            if item["is_pegin"] == true || item["is_peg_in"] == true {
                self.finding("native_pegin_boundary",vec![id.clone()],"A Liquid peg-in input crosses a protocol boundary; no same-chain parent or ownership link is inferred from its reference.",true,vec![evidence.clone()])?;
                continue;
            }
            let Some((parent, vout)) = point(item) else {
                if item["is_coinbase"] == true
                    || matches!(&item["coinbase"], Value::String(v) if v.code_points().next().is_some())
                {
                    coinbase = true;
                } else {
                    valid_inputs = false;
                }
                continue;
            };
            let prevout = if item["prevout"].is_object() {
                item["prevout"].clone()
            } else {
                json!({})
            };
            let hint = if source.trusted_reference {
                item["reference_occurrence_id"].clone()
            } else {
                Value::Null
            };
            let output = self.output(
                &chain,
                &network,
                &parent,
                &vout,
                &prevout,
                &evidence,
                rank,
                &source,
                &hint,
                !(source.trusted_reference && source.occurrence_id.is_some() && hint.is_null()),
            )?;
            inputs.push(output.clone());
            self.edge(&output, &id, "spends", &evidence, "", "observed")?;
        }
        let vout = array(&raw["vout"]);
        let mut valid_outputs = !vout.is_empty();
        for (position, item) in vout.iter().enumerate() {
            if !item.is_object() {
                valid_outputs = false;
                continue;
            }
            let Some(number) = integer(item.get("n").unwrap_or(&json!(position))) else {
                valid_outputs = false;
                continue;
            };
            if crate::analysis_features::value_sats(&item.serde()).is_none()
                && script(item).is_none()
                && !truth(&item["valuecommitment"])
            {
                valid_outputs = false;
            }
            outputs.push(self.output(
                &chain,
                &network,
                &txid,
                &number,
                item,
                &evidence,
                rank,
                &source,
                &occurrence,
                true,
            )?);
        }
        let input_set: BTreeSet<_> = inputs.iter().cloned().collect();
        let output_set: BTreeSet<_> = outputs.iter().cloned().collect();
        let complete = valid_inputs
            && valid_outputs
            && input_set.len() == inputs.len()
            && output_set.len() == outputs.len();
        if complete {
            self.shapes
                .entry(id.clone())
                .or_default()
                .insert(digest(&json!([inputs, outputs, coinbase]))?);
            self.complete_members
                .entry(id.clone())
                .or_insert_with(|| (input_set.clone(), output_set.clone()));
            if self.shapes[&id].len() > 1 {
                self.nodes.get_mut(&id).unwrap()["status"] = json!("conflicting");
                self.finding(
                    "conflicting_transaction_shape",
                    vec![id.clone()],
                    "Complete local transaction shapes disagree; physical traversal stops here.",
                    false,
                    vec![],
                )?;
            }
        }
        facts["complete"] = json!(facts["complete"] == true || complete);
        for (side, values) in [("inputs", input_set), ("outputs", output_set)] {
            let ids: BTreeSet<_> = array(&facts[side])
                .iter()
                .map(|v| text(&v["output_id"]))
                .chain(values)
                .collect();
            facts[side] = json!(ids
                .into_iter()
                .map(|v| json!({"output_id":v,"amount_msat":null}))
                .collect::<Vec<_>>());
        }
        if self
            .complete_members
            .get(&id)
            .is_some_and(|(inputs, outputs)| {
                [("inputs", inputs), ("outputs", outputs)]
                    .iter()
                    .any(|(side, values)| {
                        array(&facts[*side])
                            .iter()
                            .any(|v| !values.contains(&text(&v["output_id"])))
                    })
            })
        {
            self.nodes.get_mut(&id).unwrap()["status"] = json!("conflicting");
            self.finding("partial_observation_conflicts_with_complete_shape",vec![id.clone()],"A partial observation names inputs or outputs absent from a complete observation. They cannot form one synthetic transaction.",false,vec![evidence])?;
        }
        self.tx_facts.insert(id, facts);
        Ok(())
    }

    pub fn inventory(&mut self, row: &Value) -> Result<(), AppError> {
        validate_mappings(row, &["raw_json", "wallet_config_json", "config_json"])?;
        if !self.occurrences.accepts(&py(row), false) {
            self.invalid(false);
            return Ok(());
        }
        let Ok((chain, network)) = scope(row) else {
            self.invalid(false);
            return Ok(());
        };
        let (Some(txid), Some(vout)) = (canonical(&row["txid"]), integer(&row["vout"])) else {
            self.invalid(false);
            return Ok(());
        };
        if !["bitcoin", "liquid"].contains(&chain.as_str()) {
            self.invalid(false);
            return Ok(());
        }
        let evidence = reference_value(
            "wallet_inventory",
            &if truth(&row["id"]) {
                text(&row["id"])
            } else {
                digest(row)?
            },
            "wallet_observed",
        );
        let entry = json!({"scriptpubkey":row["script_pubkey"],"address":row["address"],"asset":row["asset"]});
        let source = self.occurrences.source(&py(row), &chain, &network)?;
        let id = self.output(
            &chain,
            &network,
            &txid,
            &vout,
            &entry,
            &evidence,
            3,
            &source,
            &Value::Null,
            true,
        )?;
        self.merge(
            &id,
            &[(
                "amount_msat",
                json!(integer(&row["amount"]).map(|v| v.to_string())),
            )],
            3,
            &evidence,
        )?;
        if nonnegative(&row["confirmations"]) {
            self.merge(
                &id,
                &[("confirmations", row["confirmations"].clone())],
                3,
                &evidence,
            )?;
        }
        self.own(&id, &row["wallet_id"]);
        self.seeds.insert(id.clone());
        let facts = self.output_facts.get_mut(&id).unwrap();
        let count = facts["inventory_observation_count"].as_u64().unwrap_or(0) + 1;
        facts["inventory_observation_count"] = json!(count);
        facts.as_object_mut().unwrap().extend(
            branch(&row["branch_label"], &row["branch_index"])
                .as_object()
                .unwrap()
                .clone(),
        );
        facts["ownership_known"] = json!(count == 1);
        facts["ownership_ambiguous"] = json!(count > 1);
        if count > 1 {
            facts.as_object_mut().unwrap().extend(json!({"branch_role":"unknown","branch_evidence_level":"unknown","change_evidence":"unavailable","branch_source":"ambiguous_inventory_ownership"}).as_object().unwrap().clone());
        }
        if let Some(spent) = canonical(&row["spent_by"]) {
            let target = self.occurrences.transaction_id(
                &chain,
                &network,
                &spent,
                &source,
                &kassiber_pyjson::Value::Null,
                true,
            )?;
            self.node(
                &target,
                Some(&chain),
                Some(&network),
                "transaction",
                Some(&spent),
            );
            self.alias(&json!(spent), &target);
            self.edge(&id, &target, "spends", &evidence, "", "wallet_observed")?;
            if !self.tx_facts.contains_key(&target) {
                self.orders
                    .entry("transaction_facts")
                    .or_default()
                    .push(target.clone());
            }
            let facts = self.tx_facts.entry(target).or_insert_with(fact);
            if facts["complete"] != true
                && !array(&facts["inputs"]).iter().any(|v| v["output_id"] == id)
            {
                facts["inputs"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"output_id":id,"amount_msat":null}));
            }
        }
        Ok(())
    }

    pub fn reference(&mut self, row: &Value, acquired: bool) -> Result<(), AppError> {
        validate_mappings(row, &["payload_json", "status_json"])?;
        let Some(payload) = parsed_mapping(&row["payload_json"], false) else {
            self.invalid(!acquired);
            return Ok(());
        };
        let mut checked = row.clone();
        checked["raw_json"] = payload.clone();
        if !self.occurrences.accepts(&py(&checked), !acquired) {
            self.invalid(!acquired);
            return Ok(());
        }
        let scope = kassiber_custody::evidence::scope(
            &json!({"chain":row["chain"],"network":row["network"]}).serde(),
        );
        let Some((chain, network, txid)) = scope
            .ok()
            .and_then(|(chain, network)| canonical(&row["txid"]).map(|txid| (chain, network, txid)))
            .filter(|(chain, network, txid)| {
                ["bitcoin", "liquid"].contains(&chain.as_str())
                    && kassiber_custody::evidence::scope(
                        &json!({"chain":chain,"network":network,"raw_json":payload}).serde(),
                    )
                    .is_ok_and(|v| v == (chain.clone(), network.clone()))
                    && canonical(&payload["txid"]).is_none_or(|id| id == *txid)
                    && (acquired
                        || crate::publication::row::py_eq(
                            &py(&row["schema_version"]),
                            &kassiber_pyjson::Value::Int(1.into()),
                        ) && payload["vin"].is_array()
                            && payload["vout"].is_array())
            })
        else {
            self.invalid(!acquired);
            return Ok(());
        };
        if truth(&row["grant_id"]) {
            let occurrence = or_text(&row["occurrence_id"]);
            let status = mapping(&row["status_json"], false);
            if let Some((block, index)) = occurrence.split_once(':').filter(|(block, index)| {
                block.len() == 64
                    && block
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    && !index.is_empty()
                    && index.bytes().all(|b| b.is_ascii_digit())
            }) {
                let _ = index;
                if status["block_hash"] != block {
                    self.invalid(false);
                    return Ok(());
                }
            }
        }
        let mut reference = format!(
            "{}:{chain}:{network}:{txid}",
            if acquired { "acquired" } else { "cache" }
        );
        if truth(&row["grant_id"]) {
            reference += &format!(
                ":{}:{}",
                text(&row["grant_id"]),
                text(&row["occurrence_id"])
            );
        }
        let source_row = json!({"id":reference,"chain":chain,"network":network,"external_id":txid,"raw_json":payload,"_index_domain_id":row["domain_id"],"_index_occurrence_id":row["occurrence_id"],"_index_reference":truth(&row["grant_id"])});
        let source = self
            .occurrences
            .source(&py(&source_row), &chain, &network)?;
        let id = self.occurrences.transaction_id(
            &chain,
            &network,
            &txid,
            &source,
            &py(&json!(source.occurrence_id)),
            true,
        )?;
        self.transaction(&source_row, true)?;
        if acquired && self.nodes.contains_key(&id) {
            self.seeds.insert(id.clone());
            let status = mapping(&row["status_json"], false);
            self.nodes.get_mut(&id).unwrap()["evidence"].as_array_mut().unwrap().push(json!({"source":"local_acquisition","reference":id,"level":"reference","observed_at":row["observed_at"],"source_name":row["source_name"],"confirmed":status["confirmed"]==true,"block_height":integer(&status["block_height"]).map(Value::Int),"status_commitment":digest(&status)?}));
            if status["removed"] == true || status["conflicted"] == true {
                self.nodes.get_mut(&id).unwrap()["status"] = json!("stale");
            }
        }
        Ok(())
    }

    pub fn relations(
        &mut self,
        decisions: &[Value],
        economics: &[Value],
        fresh: bool,
    ) -> Result<(), AppError> {
        self.coverage["custody_fresh"] = json!(fresh);
        for (table, rows, key) in [
            ("journal_custody_decisions", decisions, "decision_id"),
            (
                "journal_custody_economic_relations",
                economics,
                "relation_id",
            ),
        ] {
            for row in rows {
                let source = self
                    .records
                    .get(&text(&row["source_transaction_id"]))
                    .cloned();
                let target = self
                    .records
                    .get(&text(&row["target_transaction_id"]))
                    .cloned();
                let reference = text(&row[key]);
                let level = if row["state"] == "internal_verified" {
                    "native_verified"
                } else {
                    "reviewed"
                };
                let evidence = reference_value(table, &reference, level);
                let Some(source) = source else {
                    self.finding(
                        "missing_custody_anchor",
                        vec![],
                        "A retained custody relation is missing its source observation.",
                        false,
                        vec![evidence],
                    )?;
                    continue;
                };
                let missing_target = target.is_none();
                let target = target.unwrap_or_else(|| format!("record:custody:{reference}"));
                if missing_target {
                    self.node(&target, None, None, "record", None);
                    let node = self.nodes.get_mut(&target).unwrap();
                    node["label"] = if truth(&row["relation_kind"]) {
                        row["relation_kind"].clone()
                    } else {
                        json!("Missing custody endpoint")
                    };
                    node["status"] = json!("missing");
                    node["evidence"] = json!([evidence]);
                }
                let amount = if table == "journal_custody_decisions" {
                    integer(&row["source_start_msat"])
                        .zip(integer(&row["source_end_msat"]))
                        .filter(|(start, end)| end >= start)
                        .map(|(start, end)| (end - start).to_string())
                } else {
                    integer(&row["source_amount_msat"]).map(|v| v.to_string())
                };
                let (edge_id, created) =
                    self.edge(&source, &target, "custody", &evidence, &reference, level)?;
                let edge = self.edges.get_mut(&edge_id).unwrap();
                if created {
                    edge["amount_msat"] = json!(amount);
                    edge["asset"] = row["source_asset"].clone();
                    edge["status"] = json!(if fresh { "observed" } else { "stale" });
                }
                edge["label"] = if truth(&row["relation_kind"]) {
                    row["relation_kind"].clone()
                } else if truth(&row["state"]) {
                    row["state"].clone()
                } else {
                    json!("custody")
                };
                edge["relation_id"] = json!(reference);
                edge["basis_state"] = row["basis_state"].clone();
                edge["target_asset"] = row["target_asset"].clone();
                if table == "journal_custody_economic_relations" {
                    edge["target_amount_msat"] =
                        json!(integer(&row["target_amount_msat"]).map(|v| v.to_string()));
                }
                self.alias(&json!(reference), &source);
                self.alias(&json!(reference), &target);
                if truth(&row["component_id"]) {
                    self.alias(&row["component_id"], &source);
                    self.alias(&row["component_id"], &target);
                }
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<AnalysisIndex, AppError> {
        for id in self.occurrences.ambiguous().clone() {
            if !self.nodes.contains_key(&id) {
                continue;
            }
            self.nodes.get_mut(&id).unwrap()["status"] = json!("conflicting");
            self.finding("ambiguous_transaction_occurrence",vec![id.clone()],"The retained transaction identifier names multiple block occurrences; select an occurrence before tracing historical outputs.",false,vec![])?;
            let prefix = format!(
                "{}:out:{}:",
                id.rsplit_once(":tx:").unwrap().0,
                text(&self.nodes[&id]["txid"])
            );
            for (output, node) in &mut self.nodes {
                if output.starts_with(&prefix) {
                    node["status"] = json!("conflicting");
                }
            }
        }
        let mut outgoing = BTreeMap::<String, BTreeSet<String>>::new();
        let mut incoming = BTreeMap::<String, BTreeSet<String>>::new();
        let mut spends = BTreeMap::<String, Vec<String>>::new();
        let mut spend_order = Vec::new();
        for id in &self.edge_order {
            let edge = self.edges.get_mut(id).unwrap();
            let source = text(&edge["source"]);
            let target = text(&edge["target"]);
            if !outgoing.contains_key(&source) {
                self.orders
                    .entry("outgoing")
                    .or_default()
                    .push(source.clone());
            }
            if !incoming.contains_key(&target) {
                self.orders
                    .entry("incoming")
                    .or_default()
                    .push(target.clone());
            }
            outgoing
                .entry(source.clone())
                .or_default()
                .insert(id.clone());
            incoming
                .entry(target.clone())
                .or_default()
                .insert(id.clone());
            if matches!(edge["kind"].as_str(), Some("creates" | "spends")) {
                let output = &self.nodes[if edge["kind"] == "creates" {
                    &target
                } else {
                    &source
                }];
                edge["amount_msat"] = output["amount_msat"].clone();
                edge["asset"] = output["asset"].clone();
            }
            if edge["kind"] == "spends" {
                if self.nodes[&target]["status"] == "stale" {
                    edge["status"] = json!("stale");
                } else {
                    if !spends.contains_key(&source) {
                        spend_order.push(source.clone());
                    }
                    spends.entry(source).or_default().push(id.clone());
                }
            }
        }
        for output in spend_order {
            let edges = &spends[&output];
            let targets: BTreeSet<_> = edges
                .iter()
                .map(|id| text(&self.edges[id]["target"]))
                .collect();
            if targets.len() > 1 {
                self.nodes.get_mut(&output).unwrap()["status"] = json!("conflicting");
                for edge in edges {
                    self.edges.get_mut(edge).unwrap()["status"] = json!("conflicting");
                }
                self.finding("competing_spends",std::iter::once(output).chain(targets).collect(),"Multiple retained transactions spend the same outpoint. No current winner is inferred.",false,vec![])?;
            }
        }
        for (id, fact) in &mut self.tx_facts {
            for key in ["transaction_ids", "directions"] {
                fact[key].as_array_mut().unwrap().sort_by_key(text);
            }
            let mut available = true;
            let mut sums = [BigInt::from(0), BigInt::from(0)];
            let mut conflict = matches!(
                self.nodes[id]["status"].as_str(),
                Some("conflicting" | "stale")
            );
            for (i, side) in ["inputs", "outputs"].iter().enumerate() {
                for item in fact[*side].as_array_mut().unwrap() {
                    let output = &self.nodes[&text(&item["output_id"])];
                    item["amount_msat"] = output["amount_msat"].clone();
                    conflict |= matches!(output["status"].as_str(), Some("conflicting" | "stale"));
                    if let Some(amount) = item["amount_msat"]
                        .as_str()
                        .and_then(|v| v.parse::<BigInt>().ok())
                    {
                        sums[i] += &amount;
                    } else {
                        available = false;
                    }
                }
            }
            if conflict {
                fact["complete"] = json!(false);
            }
            let fee = &sums[0] - &sums[1];
            if fact["complete"] == true
                && !array(&fact["inputs"]).is_empty()
                && available
                && fee >= BigInt::from(0)
                && self.nodes[id]["chain"] == "bitcoin"
            {
                fact["fee_msat"] = json!(fee.to_string());
            }
        }
        let mut confirmations = ConfirmationOverlay::read(self.conn, &self.profile)?;
        for node in self.nodes.values_mut() {
            *node = json(&confirmations.apply(&py(node))?)?;
            node["wallet_ids"].as_array_mut().unwrap().sort_by_key(text);
            node["evidence"]
                .as_array_mut()
                .unwrap()
                .sort_by_key(|v| (text(&v["source"]), text(&v["reference"])));
        }
        self.coverage.as_object_mut().unwrap().extend(json!({"node_count":self.nodes.len(),"edge_count":self.edges.len(),"missing_node_count":self.nodes.values().filter(|v|v["status"]=="missing").count(),"conflicting_node_count":self.nodes.values().filter(|v|v["status"]=="conflicting").count(),"complete_transaction_count":self.tx_facts.values().filter(|v|v["complete"]==true).count(),"reference_node_count":self.nodes.values().filter(|v|v["status"]=="reference").count()}).as_object().unwrap().clone());
        let mut labels = self.labels;
        for label in &mut labels {
            let ids = self.subjects.get(&string(&label["subject"]));
            label["node_ids"] = json!(ids
                .into_iter()
                .flatten()
                .filter(|id| {
                    self.nodes[*id]["chain"] == label["chain"]
                        && self.nodes[*id]["network"] == label["network"]
                })
                .collect::<Vec<_>>());
        }
        let snapshot_id = digest(&json!([
            self.profile,
            self.nodes.iter().collect::<Vec<_>>(),
            self.edges.iter().collect::<Vec<_>>(),
            self.coverage,
            self.tx_facts.iter().collect::<Vec<_>>(),
            self.output_facts.iter().collect::<Vec<_>>(),
            labels
        ]))?;
        let mut snapshot = py(
            &json!({"snapshot_id":snapshot_id,"nodes":self.nodes,"edges":self.edges,"outgoing":outgoing,"incoming":incoming,"subjects":self.subjects,"transaction_facts":self.tx_facts,"output_facts":self.output_facts,"findings":self.findings,"coverage":self.coverage,"labels":labels,"profile_seeds":self.seeds}),
        );
        self.orders.insert("edges", self.edge_order);
        let subjects = kassiber_pyjson::Value::Dict(
            self.subject_order
                .into_iter()
                .map(|key| {
                    let ids = &self.subjects[&key];
                    (
                        kassiber_pyjson::Key::Str(key),
                        py(&json!(ids.iter().collect::<Vec<_>>())),
                    )
                })
                .collect(),
        );
        kassiber_envelope::set(&mut snapshot, "subjects", subjects);
        for (name, order) in self.orders {
            let mut values: BTreeMap<_, _> = super::fields(super::field(&snapshot, name))
                .map(|(key, value)| (key.to_owned(), value.clone()))
                .collect();
            let ordered = kassiber_pyjson::Value::Dict(
                order
                    .into_iter()
                    .map(|key| {
                        let value = values.remove(&key).expect("recorded insertion order");
                        (key.into(), value)
                    })
                    .collect(),
            );
            kassiber_envelope::set(&mut snapshot, name, ordered);
        }
        AnalysisIndex::from_snapshot(snapshot)
    }
}
