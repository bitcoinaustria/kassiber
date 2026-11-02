//! Explaining diagnostics for the private finalized-projection compiler.
//! A diagnostic never grants ownership, basis, or permission to book a row.
use super::*;
use crate::graph::stored;
use kassiber_pyjson::PyStr;
use std::collections::BTreeMap;

type Key = (PyStr, PyStr);
fn py_string(value: &Py) -> PyStr {
    match value {
        Py::Str(s) => s.clone(),
        _ => python_value_string(value).into(),
    }
}
fn field(value: &Py, key: &str) -> Result<PyStr, String> {
    let Py::Dict(entries) = value else {
        return Err("TypeError".into());
    };
    entries
        .iter()
        .find(|(k, _)| matches!(k, kassiber_pyjson::Key::Str(s) if s.is(key)))
        .map(|(_, v)| py_string(v))
        .ok_or_else(|| "KeyError".into())
}
fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn rank(reason: &str) -> (usize, &str) {
    let priority = [
        "custody_gap_review_required",
        "implicit_wallet_delta_unallocated",
        "unclaimed_source_residual",
        "source_overlap_quantity_unresolved",
        "search_capacity_incomplete",
        "capacity_source_suspense_required",
    ];
    (
        priority.iter().position(|r| *r == reason).unwrap_or(
            if reason.starts_with("custody_component_") {
                6
            } else {
                7
            },
        ),
        reason,
    )
}

pub(super) struct Quarantines<'a> {
    input: &'a super::composition::Input<'a>,
    rows: &'a dyn Rows,
    records: BTreeMap<Key, Py>,
    interpreter: BTreeSet<Key>,
}
impl<'a> Quarantines<'a> {
    pub(super) fn with_interpreter(
        input: &'a super::composition::Input<'a>,
        closure: &Value,
        diagnostics: &[Py],
        rows: &'a dyn Rows,
    ) -> Result<Self, String> {
        let mut result = Self {
            input,
            rows,
            records: BTreeMap::new(),
            interpreter: BTreeSet::new(),
        };
        for item in diagnostics {
            let raw_id = stored::get(item, "transaction_id");
            let id = if stored::truth(raw_id) {
                py_string(raw_id)
            } else {
                "".into()
            };
            let raw_reason = stored::get(item, "reason");
            let reason = if stored::truth(raw_reason) {
                py_string(raw_reason)
            } else {
                "custody_interpreter_blocked".into()
            };
            if id.code_points().next().is_some() {
                let key = (id, reason);
                result.records.insert(key.clone(), item.clone());
                result.interpreter.insert(key);
            }
        }
        let explicit: BTreeSet<_> = input
            .blocked_transaction_ids
            .iter()
            .filter(|v| codec::truth(v))
            .map(string)
            .collect();
        for id in array(&closure["blocked"]).iter().map(string) {
            if !explicit.contains(&id) && rows.get(&id).is_some() {
                result.insert(
                    &id,
                    "transfer_pair_dependency_blocked",
                    json!({
                        "required_for":"complete_transfer_component",
                        "blocked_by_transaction_ids":closure["origins"][&id]
                    }),
                    false,
                )?;
            }
        }
        Ok(result)
    }
    fn insert(&mut self, id: &str, reason: &str, detail: Value, keep: bool) -> Result<(), String> {
        let key = (id.into(), reason.into());
        let profile = &*self.input.profile;
        // Preserve Python's mandatory profile fields and json.dumps spacing.
        let workspace = profile.get("workspace_id").ok_or("KeyError")?;
        let profile_id = profile.get("id").ok_or("KeyError")?;
        let detail = kassiber_pyjson::dumps(
            &codec::from_json(&detail)?,
            &kassiber_pyjson::Options::new().sort_keys(true),
        )
        .map_err(|_| "TypeError")?;
        // Python evaluates the setdefault value even when the key exists.
        if keep && self.records.contains_key(&key) {
            return Ok(());
        }
        self.records.insert(
            key,
            codec::from_json(&json!({"transaction_id":id,"workspace_id":workspace,
            "profile_id":profile_id,"reason":reason,"detail_json":detail}))?,
        );
        Ok(())
    }
    pub(super) fn basis(&mut self, source: &Value) -> Result<(), String> {
        let id = string(&source["anchor_transaction_id"]);
        if self.rows.get(&id).is_none() {
            return Ok(());
        }
        let implausible = self.input.issues.iter().any(|issue| {
            issue["issue_type"] != "custody_interpreter_blocked"
                && issue["reason"] == "transfer_fee_implausible"
                && array(&issue["transaction_ids"]).iter().any(|v| v == &id)
        });
        let reason = if implausible {
            "transfer_fee_implausible"
        } else {
            "custody_basis_barrier"
        };
        let own = super::composition::pool(source);
        let roots = array(&self.input.pool_barrier_roots)
            .iter()
            .find(|pair| pair[0] == own)
            .map(|pair| pair[1].clone())
            .unwrap_or(json!([]));
        self.insert(
            &id,
            reason,
            json!({"required_for":"resolved_prior_custody_basis",
            "barrier_event":super::composition::barrier(source,self.input),
            "source_quantity_hash":source["quantity_hash"],"root_transaction_ids":roots}),
            true,
        )
    }
    pub(super) fn payout(&mut self, source: &Value, payout: &Value) -> Result<(), String> {
        self.insert(
            &string(&source["anchor_transaction_id"]),
            "at_swap_price_required",
            json!({"required_for":"direct_payout_carrying_value","payout_id":payout["id"]}),
            true,
        )
    }
    pub(super) fn finish<R: super::row::Fields>(
        &mut self,
        rows: &[R],
        receipts: &[Value],
        observations: &BTreeMap<String, &Value>,
    ) -> Result<(), String> {
        let mut groups: BTreeMap<(String, String), Vec<&Value>> = BTreeMap::new();
        for issue in self.input.issues.iter() {
            if issue["issue_type"] == "custody_interpreter_blocked" {
                continue;
            }
            for id in array(&issue["transaction_ids"]).iter().map(string) {
                if self.rows.get(&id).is_none() {
                    continue;
                }
                let reason = if issue["reason"] == "transfer_fee_implausible" {
                    "transfer_fee_implausible"
                } else {
                    "custody_quantity_unresolved"
                };
                groups.entry((id, reason.into())).or_default().push(issue);
            }
        }
        for ((id, reason), issues) in groups {
            if self
                .records
                .contains_key(&(id.as_str().into(), reason.as_str().into()))
            {
                continue;
            }
            let issue = *issues
                .iter()
                .min_by(|a, b| {
                    (rank(a["reason"].as_str().unwrap()), string(&a["issue_id"]))
                        .cmp(&(rank(b["reason"].as_str().unwrap()), string(&b["issue_id"])))
                })
                .expect("nonempty issue group");
            let mut detail = json!({"blocker_code":issue["reason"],
                "required_for":"finalized_custody_tax_projection","issue_id":issue["issue_id"]});
            if issues.len() > 1 {
                detail["blocker_codes"] = json!(issues
                    .iter()
                    .map(|i| string(&i["reason"]))
                    .collect::<BTreeSet<_>>());
                detail["issue_ids"] = json!(issues
                    .iter()
                    .map(|i| string(&i["issue_id"]))
                    .collect::<BTreeSet<_>>());
            }
            let gaps: BTreeSet<_> = issues
                .iter()
                .filter_map(|i| {
                    let gap = &i["details"]["gap_id"];
                    codec::truth(gap).then(|| string(gap))
                })
                .collect();
            if !gaps.is_empty() {
                detail["gap_ids"] = json!(gaps);
            }
            if text(&issue["reason"]).starts_with("custody_component_")
                || issue["issue_type"] == "component_claim_compile_failed"
            {
                detail["resolution"] = json!(
                    "review the authored component evidence and create or supersede the revision"
                );
            }
            self.insert(&id, &reason, detail, false)?;
        }
        if receipts.is_empty() {
            return Ok(());
        }
        let projected: BTreeSet<_> = rows
            .iter()
            .map(|r| text(r.field("journal_transaction_id")))
            .collect();
        let quarantined: BTreeSet<PyStr> = self.records.keys().map(|k| k.0.clone()).collect();
        for hash in receipts {
            let observation = observations.get(&string(hash)).ok_or("KeyError")?;
            let id = string(&observation["anchor_transaction_id"]);
            if !projected.contains(&id) && !quarantined.contains(&PyStr::from(id.as_str())) {
                self.basis(observation)?;
            }
        }
        Ok(())
    }
    pub(super) fn sorted(self) -> Result<Vec<Py>, String> {
        let mut records = Vec::new();
        for value in self.records.into_values() {
            let id = field(&value, "transaction_id")?;
            let reason = field(&value, "reason")?;
            let key = (id.clone(), reason.clone());
            records.push(((id, !self.interpreter.contains(&key), reason), value));
        }
        records.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(records.into_iter().map(|(_, v)| v).collect())
    }
}
