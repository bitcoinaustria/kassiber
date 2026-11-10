//! The ordered source repair and warning projections. No observation or I/O.
use super::sources::{
    self, addresses, canonical, index, lower, mapping, rows, script, text, Index,
};
use crate::{array, truth, Refusal};
use kassiber_store::rusqlite::{
    params, params_from_iter,
    types::{Value as Sql, ValueRef},
    Connection,
};
use kassiber_wallets::config::bitcoin_address_script;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const REASON: &str = "Auto-resolved overlapping wallet sources: descriptor/xpub source kept canonical; duplicate address-list transaction excluded.";
const HINT: &str = "Reopen or supersede the custody review before excluding this transaction.";

fn output_script(v: &Value) -> Option<String> {
    let nested = &v["scriptPubKey"];
    let raw = v
        .get("scriptpubkey")
        .filter(|v| !v.is_null())
        .or_else(|| v.get("script_hex").filter(|v| !v.is_null()))
        .or_else(|| {
            if nested.is_object() {
                Some(if truth(&nested["hex"]) {
                    &nested["hex"]
                } else {
                    &nested["scriptpubkey"]
                })
            } else {
                nested.as_str().map(|_| nested)
            }
        });
    raw.and_then(script)
}
fn transaction_scripts(row: &Value) -> Result<BTreeSet<String>, Refusal> {
    let mut raw = mapping(&row["raw_json"]);
    if raw["tx"].is_object() {
        raw = raw["tx"].clone();
    }
    let direction = lower(&text(&row["direction"]));
    let mut scripts = BTreeSet::new();
    let entries = |key: &str| -> Result<&[Value], Refusal> {
        let v = &raw[key];
        if truth(v) && (v.is_boolean() || v.is_number()) {
            let kind = if v.is_boolean() {
                "bool"
            } else if v.as_number().is_some_and(|n| n.is_i64() || n.is_u64()) {
                "int"
            } else {
                "float"
            };
            return Err(Refusal::new(
                "TypeError",
                format!("'{kind}' object is not iterable"),
            ));
        }
        Ok(array(v))
    };
    if direction.is_empty() || direction == "inbound" {
        scripts.extend(entries("vout")?.iter().filter_map(output_script));
    }
    if direction.is_empty() || direction == "outbound" {
        scripts.extend(
            entries("vin")?
                .iter()
                .filter_map(|v| output_script(&v["prevout"])),
        );
    }
    Ok(scripts)
}

/// `row["amount"] or 0` in the duplicate key: the stored value itself, grouped
/// as Python's `==` groups it (`5 == 5.0`), so a fraction or text is never
/// truncated into another record's quantity (41978c2df).
fn economic_key(v: ValueRef<'_>) -> Result<String, Refusal> {
    Ok(match v {
        ValueRef::Null => "n0".into(),
        ValueRef::Integer(n) => format!("n{n}"),
        ValueRef::Real(0.0) => "n0".into(),
        ValueRef::Real(n) if n.is_finite() && n.fract() == 0.0 => format!("n{n:.0}"),
        ValueRef::Real(n) => format!("f{}", n.to_bits()),
        ValueRef::Text([]) => "n0".into(),
        ValueRef::Text(raw) => format!(
            "s{}",
            std::str::from_utf8(raw).map_err(|_| Refusal::new("ValueError", "invalid UTF-8"))?
        ),
        ValueRef::Blob([]) => "n0".into(),
        ValueRef::Blob(raw) => format!(
            "b{}",
            raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ),
    })
}

/// `int(value)` over a stored amount or fee; only its refusal matters.
fn stored_int(v: ValueRef<'_>) -> Result<(), Refusal> {
    let v = sources::value(v)?;
    if let Some(text) = v.as_str() {
        kassiber_pytext::parse_int(text).map_err(|error| {
            let message = match error {
                kassiber_pytext::NumberError::Invalid => format!(
                    "invalid literal for int() with base 10: {}",
                    kassiber_pytext::repr_str(text)
                ),
                kassiber_pytext::NumberError::TooManyDigits(n) => format!(
                    "Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"
                ),
            };
            Refusal::new("ValueError", message)
        })?;
        return Ok(());
    }
    kassiber_custody::codec::integer(&v)
        .map(drop)
        .map_err(Refusal::from)
}

fn preview(conn: &Connection, profile: &str, idx: &Index) -> Result<Value, Refusal> {
    limited_preview(conn, profile, idx, None)
}

/// `duplicate_transaction_preview(..., limit=limit)`: groups past the limit are
/// counted but neither shown nor recommended for exclusion.
fn limited_preview(
    conn: &Connection,
    profile: &str,
    idx: &Index,
    limit: Option<usize>,
) -> Result<Value, Refusal> {
    let overlaps = idx.overlaps();
    let mut wallet_ids = BTreeSet::new();
    let mut canonical_ids = BTreeSet::new();
    let mut scripts = HashMap::<String, BTreeSet<String>>::new();
    for overlap in &overlaps {
        canonical_ids.insert(text(&overlap["recommended_canonical_wallet_id"]));
        wallet_ids.extend(array(&overlap["wallets"]).iter().map(|w| text(&w["id"])));
    }
    if wallet_ids.len() < 2 {
        return Ok(json!({"duplicate_groups":[],"recommended_exclusions":[],"limited":false}));
    }
    for (_, group) in &idx.groups {
        if group
            .iter()
            .map(|s| &s.wallet)
            .collect::<BTreeSet<_>>()
            .len()
            < 2
        {
            continue;
        }
        for s in group {
            scripts
                .entry(s.wallet.clone())
                .or_default()
                .insert(s.script.clone());
        }
    }
    let sql = format!("SELECT id,wallet_id,external_id,occurred_at,direction,asset,amount,fee,raw_json FROM transactions WHERE profile_id=? AND excluded=0 AND wallet_id IN ({}) AND external_id IS NOT NULL ORDER BY occurred_at ASC,created_at ASC,id ASC", vec!["?"; wallet_ids.len()].join(","));
    let args = std::iter::once(profile.to_string())
        .chain(wallet_ids)
        .map(Sql::Text);
    let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
    let mut positions = HashMap::new();
    let mut stmt = conn.prepare(&sql)?;
    let mut cursor = stmt.query(params_from_iter(args))?;
    while let Some(found) = cursor.next()? {
        let mut row = serde_json::Map::new();
        for (i, name) in [
            "id",
            "wallet_id",
            "external_id",
            "occurred_at",
            "direction",
            "asset",
        ]
        .into_iter()
        .enumerate()
        {
            row.insert(name.into(), sources::value(found.get_ref(i)?)?);
        }
        row.insert("raw_json".into(), sources::value(found.get_ref(8)?)?);
        let (amount, fee) = (found.get_ref(6)?, found.get_ref(7)?);
        let row = Value::Object(row);
        let evidence = transaction_scripts(&row)?;
        if !scripts
            .get(&text(&row["wallet_id"]))
            .is_some_and(|s| !s.is_disjoint(&evidence))
        {
            continue;
        }
        let key = json!([
            lower(&text(&row["external_id"])),
            row["direction"],
            row["asset"],
            economic_key(amount)?,
            economic_key(fee)?
        ]);
        let i = *positions.entry(key.to_string()).or_insert_with(|| {
            groups.push((key, Vec::new()));
            groups.len() - 1
        });
        groups[i].1.push(row);
    }
    let mut duplicates = Vec::new();
    let mut exclusions = Vec::new();
    let mut duplicate_groups = 0;
    for (key, group) in groups {
        if group
            .iter()
            .map(|r| text(&r["wallet_id"]))
            .collect::<BTreeSet<_>>()
            .len()
            < 2
        {
            continue;
        }
        let Some(keep) = group
            .iter()
            .find(|r| canonical_ids.contains(&text(&r["wallet_id"])))
        else {
            continue;
        };
        let exclude: Vec<_> = group
            .iter()
            .filter(|r| !canonical_ids.contains(&text(&r["wallet_id"])))
            .map(|r| r["id"].clone())
            .collect();
        if exclude.is_empty() {
            continue;
        }
        duplicate_groups += 1;
        if limit.is_some_and(|limit| duplicates.len() >= limit) {
            continue;
        }
        duplicates.push(json!({"external_id":key[0],"occurred_at":group.iter().map(|r| text(&r["occurred_at"])).min().unwrap(),"direction":key[1],"asset":key[2],"transaction_count":group.len(),"recommended_keep_transaction_id":keep["id"],"recommended_exclude_transaction_ids":exclude}));
        exclusions.extend(exclude);
    }
    Ok(
        json!({"duplicate_groups":duplicates,"recommended_exclusions":exclusions,"limited":limit.is_some_and(|limit| duplicate_groups > limit),"repair_policy":"Preview only. Exclude duplicate transaction rows through the audited metadata exclusion path after user review; do not delete wallets or cascade-delete transactions automatically."}),
    )
}

fn unresolved(idx: &Index) -> Vec<Value> {
    let mut wallets = BTreeMap::<(String, String), Value>::new();
    for o in idx.overlaps() {
        let repair: BTreeSet<_> = array(&o["address_list_repair_preview"])
            .iter()
            .map(|w| text(&w["wallet_id"]))
            .collect();
        for w in array(&o["wallets"]) {
            let id = text(&w["id"]);
            if repair.contains(&id) && w["active_transaction_count"].as_i64().unwrap_or(0) > 0 {
                let label = text(&w["label"]);
                let row = wallets.entry((label.clone(), id.clone())).or_insert_with(
                    || json!({"wallet_id":id,"wallet":label,"active_transaction_count":0}),
                );
                row["active_transaction_count"] = json!(row["active_transaction_count"]
                    .as_i64()
                    .unwrap()
                    .max(w["active_transaction_count"].as_i64().unwrap()));
            }
        }
    }
    wallets.into_values().collect()
}

fn conflict(conn: &Connection, profile: &str, tx: &str) -> Result<Option<Value>, Refusal> {
    let reviews = rows(conn, "SELECT component.id AS component_id,term.legacy_source_id AS review_id,term.term_kind FROM custody_component_legs leg JOIN custody_components component ON component.id=leg.component_id LEFT JOIN custody_component_economic_terms term ON term.component_id=component.id AND (term.source_leg_id=leg.id OR term.target_leg_id=leg.id) WHERE component.profile_id=? AND component.state='active' AND COALESCE(leg.anchor_transaction_id,leg.transaction_id)=? ORDER BY component.created_at,component.id,term.ordinal LIMIT 1", [profile,tx])?;
    Ok(reviews.first().map(|r| {
        let id = if truth(&r["review_id"]) { &r["review_id"] } else { &r["component_id"] };
        json!({"transaction_id":tx,"reason":"conflict","message":format!("Transaction belongs to active custody review {}; excluding it would orphan reviewed custody evidence.",text(id)),"hint":HINT})
    }))
}

fn exclude(
    conn: &Connection,
    profile: &str,
    tx: &str,
    clock: &mut impl FnMut() -> Result<String, Refusal>,
    identity: &mut impl FnMut() -> Result<String, Refusal>,
) -> Result<Value, Refusal> {
    let tx = rows(conn,"SELECT t.id,t.wallet_id,t.external_id,t.occurred_at,t.excluded,w.label AS wallet_label FROM transactions t LEFT JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.id=?",[profile,tx])?.remove(0);
    let updated = !truth(&tx["excluded"]);
    let mut event = Value::Null;
    if updated {
        conn.execute(
            "UPDATE transactions SET excluded=1 WHERE id=?",
            [text(&tx["id"])],
        )?;
        let p = rows(conn, "SELECT workspace_id,journal_input_version,last_processed_input_version,last_processed_at,last_processed_tx_count FROM profiles WHERE id=?", [profile])?.remove(0);
        let at = clock()?;
        let id = identity()?;
        let field = identity()?;
        conn.execute("INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,wallet_id,transaction_external_id,transaction_occurred_at,source,reason,changed_at,journal_input_version,journal_input_version_after,last_processed_input_version,last_processed_at,last_processed_tx_count) VALUES(?,?,?,?,?,?,?,'cli',?,?,?,?,?,?,?)",params![id,text(&p["workspace_id"]),profile,text(&tx["id"]),tx["wallet_id"].as_str(),tx["external_id"].as_str(),tx["occurred_at"].as_str(),REASON,at,p["journal_input_version"].as_i64().unwrap_or(0),p["journal_input_version"].as_i64().unwrap_or(0)+1,p["last_processed_input_version"].as_i64().unwrap_or(0),p["last_processed_at"].as_str(),p["last_processed_tx_count"].as_i64().unwrap_or(0)])?;
        conn.execute("INSERT INTO transaction_edit_fields(id,event_id,field,before_value,after_value,diff_json) VALUES(?,?,'excluded','false','true','{}')",params![field,id])?;
        conn.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE id=?",[profile])?;
        event = json!(id);
    }
    // `get_transaction_record` reads the row back after the audited write, and
    // its `int()` of the stored amount, then fee, refuses there.
    let mut stmt = conn.prepare("SELECT amount,fee FROM transactions WHERE id=?")?;
    let mut cursor = stmt.query([text(&tx["id"])])?;
    if let Some(found) = cursor.next()? {
        stored_int(found.get_ref(0)?)?;
        stored_int(found.get_ref(1)?)?;
    }
    Ok(
        json!({"transaction_id":tx["id"],"wallet":text(&tx["wallet_label"]),"external_id":text(&tx["external_id"]),"history_event_id":event,"updated":updated}),
    )
}

fn trim(conn: &Connection, profile: &str, idx: &Index) -> Result<Value, Refusal> {
    let mut scripts = HashMap::<String, BTreeSet<String>>::new();
    for (key, group) in &idx.groups {
        if group
            .iter()
            .map(|s| &s.wallet)
            .collect::<BTreeSet<_>>()
            .len()
            < 2
        {
            continue;
        }
        let o = sources::payload(key, group);
        let id = text(&o["recommended_canonical_wallet_id"]);
        if !group.iter().any(|s| s.wallet == id && canonical(&s.kind)) {
            continue;
        }
        for s in group {
            if s.wallet != id && s.source == "address_list" {
                scripts
                    .entry(s.wallet.clone())
                    .or_default()
                    .insert(s.script.clone());
            }
        }
    }
    let mut updates = Vec::new();
    let mut total = 0;
    if scripts.is_empty() {
        return Ok(json!({"wallets_updated":[],"addresses_removed":0}));
    }
    for row in rows(
        conn,
        "SELECT id,label,config_json FROM wallets WHERE profile_id=?",
        [profile],
    )? {
        let id = text(&row["id"]);
        let Some(scripts) = scripts.get(&id) else {
            continue;
        };
        let mut c = mapping(&row["config_json"]);
        let (removed, kept): (Vec<_>, Vec<_>) = addresses(&row["config_json"])?
            .into_iter()
            .partition(|a| bitcoin_address_script(a).is_some_and(|s| scripts.contains(&s)));
        if removed.is_empty() {
            continue;
        }
        c["addresses"] = json!(kept);
        if kept.is_empty() {
            c["deprecated"] = json!(true);
        }
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id=?",
            params![sources::stored_config(&c)?, id],
        )?;
        let sql = format!("DELETE FROM wallet_utxos WHERE profile_id=? AND wallet_id=? AND (lower(script_pubkey) IN ({}) OR address IN ({}))",vec!["?";scripts.len()].join(","),vec!["?";removed.len()].join(","));
        let args = [profile.to_string(), id.clone()]
            .into_iter()
            .chain(scripts.iter().cloned())
            .chain(removed.iter().cloned());
        conn.execute(&sql, params_from_iter(args))?;
        conn.execute(
            "DELETE FROM freshness_source_states WHERE profile_id=? AND source_key=?",
            [profile, &format!("onchain_wallet:{id}")],
        )?;
        total += removed.len();
        updates.push(json!({"wallet_id":id,"wallet":if truth(&row["label"]) {text(&row["label"])} else {text(&row["id"])},"addresses_removed":removed.len(),"remaining_addresses":kept.len(),"deprecated":kept.is_empty(),"output_inventory_cleared":true,"refresh_checkpoint_reset":true}));
    }
    Ok(json!({"wallets_updated":updates,"addresses_removed":total}))
}

/// Repair finite source overlaps, preserving audited exclusions and reviewed
/// conflicts. The caller supplies the clock and audit identities, owns the
/// writer transaction and commits only after the whole journal pipeline succeeds.
pub fn repair_journal_source_overlaps(
    conn: &Connection,
    profile_id: &str,
    clock: &mut impl FnMut() -> Result<String, Refusal>,
    identity: &mut impl FnMut() -> Result<String, Refusal>,
) -> Result<Option<Value>, Refusal> {
    super::require_writer(conn)?;
    super::atomic(conn, "r4_journal_sources", || {
        run(conn, profile_id, index(conn, profile_id)?, clock, identity)
    })
}

pub(super) fn repair_and_warn(
    conn: &Connection,
    profile: &str,
    clock: &mut impl FnMut() -> Result<String, Refusal>,
    identity: &mut impl FnMut() -> Result<String, Refusal>,
) -> Result<(Option<Value>, Option<Value>), Refusal> {
    super::require_writer(conn)?;
    super::atomic(conn, "r4_journal_sources", || {
        let idx = index(conn, profile)?;
        if idx.overlaps().is_empty() {
            return Ok((None, None));
        }
        let repair = run(conn, profile, idx, clock, identity)?;
        let warning = source_overlap_warning(conn, profile, repair.as_ref())?;
        Ok((repair, warning))
    })
}

fn run(
    conn: &Connection,
    profile: &str,
    idx: Index,
    clock: &mut impl FnMut() -> Result<String, Refusal>,
    identity: &mut impl FnMut() -> Result<String, Refusal>,
) -> Result<Option<Value>, Refusal> {
    if idx.overlaps().is_empty() {
        return Ok(None);
    }
    let p = preview(conn, profile, &idx)?;
    let mut excluded = Vec::new();
    let mut skipped = Vec::new();
    for tx in array(&p["recommended_exclusions"]) {
        let tx = text(tx);
        if let Some(c) = conflict(conn, profile, &tx)? {
            skipped.push(c);
            continue;
        }
        excluded.push(exclude(conn, profile, &tx, clock, identity)?);
    }
    let idx = index(conn, profile)?;
    let p = preview(conn, profile, &idx)?;
    let unresolved = unresolved(&idx);
    let can_trim = skipped.is_empty()
        && array(&p["recommended_exclusions"]).is_empty()
        && unresolved.is_empty();
    let repair = if can_trim {
        trim(conn, profile, &idx)?
    } else {
        json!({"wallets_updated":[],"addresses_removed":0})
    };
    if repair["addresses_removed"] != 0
        || !excluded.is_empty()
        || !skipped.is_empty()
        || !unresolved.is_empty()
    {
        let remaining = index(conn, profile)?.overlaps();
        return Ok(Some(
            json!({"addresses_removed":repair["addresses_removed"],"wallets_updated":repair["wallets_updated"],"duplicates_excluded":excluded.iter().filter(|r|r["updated"]==true).count(),"excluded_records":excluded,"skipped_records":skipped,"address_list_trim_skipped":!can_trim,"unresolved_address_list_wallets":unresolved,"remaining_overlap_count":remaining.len(),"remaining_overlaps":remaining}),
        ));
    }
    Ok(None)
}

/// The post-repair warning, including the complete finite evidence and preview.
pub fn source_overlap_warning(
    conn: &Connection,
    profile_id: &str,
    repair: Option<&Value>,
) -> Result<Option<Value>, Refusal> {
    let idx = index(conn, profile_id)?;
    let overlap = idx.projection();
    if overlap["overlap_count"] == 0 {
        return Ok(None);
    }
    let mut details = json!({"overlap_count":overlap["overlap_count"],"overlap":overlap,"repair_preview":preview(conn,profile_id,&idx)?});
    if let Some(repair) = repair {
        details["repair_attempt"] = repair.clone();
    }
    Ok(Some(details))
}

/// The report blocker's `detect_profile_source_overlaps` result and its
/// 20-group duplicate preview, or `None` when no finite script overlaps.
pub fn source_overlap_blocker(
    conn: &Connection,
    profile_id: &str,
) -> Result<Option<(kassiber_pyjson::Value, kassiber_pyjson::Value)>, Refusal> {
    let idx = index(conn, profile_id)?;
    let overlap = idx.projection();
    if array(&overlap["overlaps"]).is_empty() {
        return Ok(None);
    }
    let preview = limited_preview(conn, profile_id, &idx, Some(20))?;
    let fields = source_overlap_result_fields(
        None,
        Some(
            &json!({"overlap_count":overlap["overlap_count"],"overlap":overlap,"repair_preview":preview}),
        ),
    )?;
    let kassiber_pyjson::Value::Dict(mut fields) = fields else {
        unreachable!("source overlap fields are a mapping")
    };
    let (_, warning) = fields.remove(0);
    let field = |key: &str| {
        kassiber_envelope::get(&warning, key)
            .cloned()
            .unwrap_or(kassiber_pyjson::Value::Null)
    };
    Ok(Some((field("overlap"), field("repair_preview"))))
}

/// Append these fields to the process result after its ordinary journal fields.
/// Python's nested dictionary order is part of the machine response bytes.
pub fn source_overlap_result_fields(
    repair: Option<&Value>,
    warning: Option<&Value>,
) -> Result<kassiber_pyjson::Value, Refusal> {
    use kassiber_envelope::dict;
    fn project(v: &Value) -> Result<kassiber_pyjson::Value, Refusal> {
        if let Some(items) = v.as_array() {
            return items
                .iter()
                .map(project)
                .collect::<Result<Vec<_>, _>>()
                .map(kassiber_pyjson::Value::List);
        }
        if !v.is_object() {
            return kassiber_custody::codec::from_json(v).map_err(Refusal::from);
        }
        let fields: &[&str] = if v.get("duplicates_excluded").is_some() {
            &[
                "addresses_removed",
                "wallets_updated",
                "duplicates_excluded",
                "excluded_records",
                "skipped_records",
                "address_list_trim_skipped",
                "unresolved_address_list_wallets",
                "remaining_overlap_count",
                "remaining_overlaps",
            ]
        } else if v.get("repair_preview").is_some() {
            &[
                "overlap_count",
                "overlap",
                "repair_preview",
                "repair_attempt",
            ]
        } else if v.get("overlaps").is_some() {
            &["overlap_count", "overlaps", "checked"]
        } else if v.get("recommended_canonical_wallet_id").is_some() {
            &[
                "chain",
                "network",
                "wallets",
                "recommended_canonical_wallet_id",
                "recommended_canonical_wallet",
                "recommendation_reason",
                "evidence",
                "address_list_repair_preview",
                "checked",
            ]
        } else if v.get("output_inventory_cleared").is_some() {
            &[
                "wallet_id",
                "wallet",
                "addresses_removed",
                "remaining_addresses",
                "deprecated",
                "output_inventory_cleared",
                "refresh_checkpoint_reset",
            ]
        } else if v.get("history_event_id").is_some() {
            &[
                "transaction_id",
                "wallet",
                "external_id",
                "history_event_id",
                "updated",
            ]
        } else if v.get("message").is_some() {
            &["transaction_id", "reason", "message", "hint"]
        } else if v.get("overlapping_address_list_target_count").is_some() {
            &[
                "wallet_id",
                "wallet",
                "overlapping_address_list_target_count",
                "action",
                "clear_output_inventory",
                "reset_onchain_refresh_checkpoint",
                "deprecate_if_empty_after_trim",
                "requires_confirmation",
            ]
        } else if v.get("sources").is_some() {
            &[
                "id",
                "label",
                "kind",
                "deprecated",
                "active_transaction_count",
                "sources",
            ]
        } else if v.get("target_count").is_some() {
            &["wallet_id", "wallet", "target_count"]
        } else if v.get("active_transaction_count").is_some() {
            &["wallet_id", "wallet", "active_transaction_count"]
        } else if v.get("source_count").is_some() {
            &[
                "scope",
                "source_count",
                "wallet_count",
                "descriptor_global_overlap_proven",
            ]
        } else if v.get("target_counts").is_some() {
            &[
                "scope",
                "descriptor_global_overlap_proven",
                "note",
                "target_counts",
            ]
        } else if v.get("duplicate_groups").is_some() {
            &[
                "duplicate_groups",
                "recommended_exclusions",
                "limited",
                "repair_policy",
            ]
        } else if v.get("recommended_keep_transaction_id").is_some() {
            &[
                "external_id",
                "occurred_at",
                "direction",
                "asset",
                "transaction_count",
                "recommended_keep_transaction_id",
                "recommended_exclude_transaction_ids",
            ]
        } else {
            return Err(Refusal::new(
                "RuntimeError",
                "unknown source-overlap result projection",
            ));
        };
        let pairs = fields
            .iter()
            .filter_map(|key| v.get(*key).map(|value| (*key, value)))
            .map(|(key, v)| Ok((key, project(v)?)))
            .collect::<Result<Vec<_>, Refusal>>()?;
        Ok(dict(pairs))
    }
    let mut fields = Vec::new();
    if let Some(v) = repair {
        fields.push(("source_overlap_repair", project(v)?));
    }
    if let Some(v) = warning {
        fields.push(("source_overlap_warning", project(v)?));
    }
    Ok(dict(fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(v: ValueRef<'_>) -> String {
        economic_key(v).unwrap()
    }

    #[test]
    fn economic_values_group_by_python_equality() {
        // `5.0 == 5`, so an integral REAL groups with the equal integer.
        assert_eq!(key(ValueRef::Real(5.0)), key(ValueRef::Integer(5)));
        // Text never equals a stored number, and a fraction stays itself.
        assert_ne!(key(ValueRef::Text(b"5")), key(ValueRef::Integer(5)));
        assert_ne!(key(ValueRef::Real(5.5)), key(ValueRef::Integer(5)));
        // `row["amount"] or 0`.
        assert_eq!(key(ValueRef::Null), key(ValueRef::Integer(0)));
        assert_eq!(key(ValueRef::Real(-0.0)), key(ValueRef::Integer(0)));
    }
}
