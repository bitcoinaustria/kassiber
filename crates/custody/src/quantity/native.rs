//! Exact native audit augmentation, isolated from production authority.
use super::{array, exact, number, principal};
use crate::evidence::canonical::{EventRef, LegRef};
use crate::fxhash::HashMap;
use crate::{
    codec,
    evidence::{self, EventKey},
    python_text::PythonText,
};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
fn text(v: &Value) -> String {
    if codec::truth(v) {
        codec::pystr(v)
    } else {
        String::new()
    }
}
fn hash(v: &Value) -> Result<String, String> {
    Ok(codec::sha256(&codec::dumps(&codec::from_json(v)?, false)?))
}
fn key(v: &Value) -> EventKey {
    EventKey {
        chain: text(&v["chain"]),
        network: text(&v["network"]),
        native_namespace: text(&v["native_namespace"]),
        native_event_id: text(&v["native_event_id"]),
    }
}
fn exact_msat(v: &Value) -> Option<BigInt> {
    exact(v).ok().filter(|n| *n >= BigInt::from(0))
}
fn issue(
    reason: &str,
    a: &Value,
    ids: Vec<String>,
    asset: Value,
    amount: Value,
    details: Value,
) -> Result<Value, String> {
    let ids: BTreeSet<_> = ids.into_iter().filter(|s| !s.is_empty()).collect();
    let id = hash(
        &json!({"schema_version":1,"kind":"native_audit_issue","reason":reason,"transaction_ids":ids,"pairing_source":text(&a["pairing_source"])}),
    )?;
    Ok(
        json!({"$type":"NativeAuditIssue","issue_id":id,"reason":reason,"occurred_at":text(&a["occurred_at"]),"transaction_ids":ids,"asset":asset,"amount_msat":amount,"details":details}),
    )
}
/// Observation per alias transaction id, borrowed; a later alias replaces an
/// earlier one and an alias without its leg is a `KeyError`.
fn observation_refs<'a>(
    events: &'a BTreeMap<EventKey, Cow<'_, Value>>,
) -> Result<HashMap<String, &'a Value>, String> {
    let by_hash: HashMap<_, _> = events
        .values()
        .flat_map(|e| array(&e["legs"]))
        .map(|o| (super::text_key(&o["quantity_hash"]), o))
        .collect();
    let mut by_tx = HashMap::default();
    for e in events.values() {
        for alias in array(&e["observation_aliases"]) {
            let o = by_hash
                .get(super::text_key(&alias[1]).as_ref())
                .ok_or("KeyError")?;
            by_tx.insert(text(&alias[0]), *o);
        }
    }
    Ok(by_tx)
}
fn observations(
    events: &BTreeMap<EventKey, Cow<'_, Value>>,
) -> Result<BTreeMap<String, Value>, String> {
    Ok(observation_refs(events)?
        .into_iter()
        .map(|(id, o)| (id, o.clone()))
        .collect())
}
fn add(event: &mut Value, o: Value, aliases: BTreeMap<String, Value>) {
    let mut legs = array(&event["legs"]).to_vec();
    legs.push(o.clone());
    legs.sort_by_cached_key(|o| {
        (
            text(&o["wallet_id"]),
            text(&o["direction"]),
            text(&o["asset"]),
            text(&o["quantity_hash"]),
        )
    });
    let mut snapshots = array(&event["evidence_snapshots"]).to_vec();
    snapshots.push(json!({"$type":"EvidenceSnapshot","quantity_hash":o["quantity_hash"],"detail_hash":o["evidence_detail_hash"],"payload_json":o["evidence_payload_json"]}));
    snapshots.sort_by_cached_key(|s| (text(&s["detail_hash"]), text(&s["quantity_hash"])));
    event["legs"] = json!(legs);
    event["evidence_snapshots"] = json!(snapshots);
    event["observation_aliases"] = json!(aliases.into_iter().collect::<Vec<_>>());
}
fn aliases(event: &Value) -> BTreeMap<String, Value> {
    array(&event["observation_aliases"])
        .iter()
        .map(|a| (text(&a[0]), a[1].clone()))
        .collect()
}
fn synthetic(row: Value, event_key: &EventKey) -> Result<Value, String> {
    let result = evidence::observation_fields(&row, event_key).map_err(|e| e.code.to_owned())?;
    super::validate_observation(&result)?;
    Ok(result)
}
#[derive(Clone)]
struct Leg {
    evidence: String,
    out: String,
    input: String,
    source_id: String,
    source: Value,
    wallet: String,
    asset: String,
    received: BigInt,
    fee: BigInt,
    pairing: String,
    time: String,
}
impl Leg {
    fn source_hash(&self) -> String {
        text(&self.source["quantity_hash"])
    }
}
fn virtual_sources(
    events: &mut BTreeMap<EventKey, Cow<'_, Value>>,
    audits: &[Value],
    issues: &mut Vec<Value>,
) -> Result<(), String> {
    let mut audits: Vec<_> = audits
        .iter()
        .filter(|a| a["virtual_source"] == true && a["pairing_source"] == "channel_lifecycle")
        .collect();
    if audits.is_empty() {
        return Ok(());
    }
    let mut by_tx = observations(events)?;
    audits.sort_by_cached_key(|a| (text(&a["occurred_at"]), text(&a["out_id"])));
    for a in audits {
        let out = text(&a["out_id"]);
        let target_id = text(&a["in_anchor_transaction_id"]);
        let target = by_tx.get(&target_id).cloned();
        let received = exact_msat(&a["crypto_received_msat"]);
        let fee = exact_msat(&a["crypto_fee_msat"]);
        let sent = exact_msat(&a["crypto_sent_msat"]);
        let wallet = text(&a["from_wallet_id"]);
        let valid = target.as_ref().is_some_and(|t| {
            t["direction"] == "inbound"
                && text(&a["asset"]).py_upper() == t["asset"]
                && text(&a["to_wallet_id"]) == t["wallet_id"]
                && text(&a["occurred_at"]) == t["occurred_at"]
        });
        if out.is_empty()
            || !valid
            || wallet.is_empty()
            || received.as_ref().is_none_or(|n| *n <= BigInt::from(0))
            || fee.is_none()
            || sent != received.as_ref().zip(fee.as_ref()).map(|(r, f)| r + f)
        {
            issues.push(issue(
                "native_audit_virtual_source_invalid",
                a,
                vec![out, target_id],
                target
                    .as_ref()
                    .map(|t| t["asset"].clone())
                    .unwrap_or(Value::Null),
                received.as_ref().map(number).unwrap_or(Value::Null),
                json!({}),
            )?);
            continue;
        }
        let t = target.unwrap();
        let r = received.unwrap();
        let f = fee.unwrap();
        let event_key = key(&t["event_key"]);
        let event = events.get_mut(&event_key).ok_or("KeyError")?.to_mut();
        let mut aliases = aliases(event);
        if let Some(prior_hash) = aliases.get(&out) {
            let valid = array(&event["legs"])
                .iter()
                .find(|o| o["quantity_hash"] == *prior_hash)
                .is_some_and(|o| {
                    o["direction"] == "outbound"
                        && o["wallet_id"] == wallet
                        && o["asset"] == t["asset"]
                        && principal(o).ok() == Some(r.clone())
                        && exact(&o["fee_msat"]).ok() == Some(f.clone())
                });
            if !valid {
                issues.push(issue(
                    "native_audit_virtual_source_contradiction",
                    a,
                    vec![out, target_id],
                    t["asset"].clone(),
                    number(&r),
                    json!({}),
                )?);
            }
            continue;
        }
        let proof = json!({"schema_version":1,"kind":"verified_channel_close_source","source_wallet_id":wallet,"target_quantity_hash":t["quantity_hash"],"received_msat":number(&r),"fee_msat":number(&f)});
        let proof_hash = hash(&proof)?;
        let o = synthetic(
            json!({"id":out,"journal_transaction_id":t["anchor_transaction_id"],"wallet_id":wallet,"direction":"outbound","asset":t["asset"],"amount":number(&r),"fee":number(&f),"amount_includes_fee":false,"occurred_at":t["occurred_at"],"fingerprint":format!("native-channel-close-out:{proof_hash}"),"kind":"self_transfer_out","raw_json":{"txid":event_key.native_event_id,"_kassiber_quantity_evidence":proof}}),
            &event_key,
        )?;
        aliases.insert(out.clone(), o["quantity_hash"].clone());
        add(event, o.clone(), aliases);
        by_tx.insert(out, o);
    }
    Ok(())
}
fn parse(
    events: &BTreeMap<EventKey, Cow<'_, Value>>,
    audits: &[Value],
    excluded: &BTreeSet<String>,
    issues: &mut Vec<Value>,
    shared: Option<&[(String, LegRef<'_>)]>,
) -> Result<Vec<Leg>, String> {
    // Shared aliases name the same JSON legs `observation_refs` finds.
    let by_tx = match shared {
        Some(aliases) if aliases.iter().all(|(_, o)| matches!(o, LegRef::Json(_))) => aliases
            .iter()
            .map(|(id, o)| match o {
                LegRef::Json(o) => (id.clone(), *o),
                LegRef::Typed(_) => unreachable!("JSON alias"),
            })
            .collect(),
        _ => observation_refs(events)?,
    };
    let mut legs = Vec::new();
    let mut audits: Vec<_> = audits
        .iter()
        .filter(|a| {
            matches!(
                text(&a["pairing_source"]).as_str(),
                "ownership_derived"
                    | "multi_source_consolidation"
                    | "recorded_fanout"
                    | "channel_lifecycle"
            )
        })
        .collect();
    audits.sort_by_cached_key(|a| {
        (
            text(&a["occurred_at"]),
            text(&a["out_id"]),
            text(&a["in_id"]),
        )
    });
    for a in audits {
        let out = text(&a["out_id"]);
        let input = text(&a["in_id"]);
        let source_id = text(&a["out_anchor_transaction_id"]);
        if excluded.contains(&source_id) {
            continue;
        }
        let ids = vec![source_id.clone(), out.clone(), input.clone()];
        let source = by_tx.get(&source_id);
        if source_id.is_empty() || source.is_none_or(|o| o["direction"] != "outbound") {
            let asset = text(&a["asset"]).py_upper();
            issues.push(issue(
                "native_audit_source_anchor_missing",
                a,
                ids,
                if asset.is_empty() {
                    Value::Null
                } else {
                    json!(asset)
                },
                Value::Null,
                json!({}),
            )?);
            continue;
        }
        let o = source.unwrap();
        let pairing = text(&a["pairing_source"]);
        if pairing != "channel_lifecycle" && !codec::truth(&o["authoritative_chain_observation"]) {
            issues.push(issue(
                "native_audit_authoritative_observation_missing",
                a,
                ids,
                o["asset"].clone(),
                Value::Null,
                json!({"authority_reason":o["observation_authority_reason"]}),
            )?);
            continue;
        }
        let k = key(&o["event_key"]);
        if !matches!(k.chain.as_str(), "bitcoin" | "liquid")
            || k.native_namespace != "chain"
            || k.native_event_id.chars().count() != 64
            || !k
                .native_event_id
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        {
            issues.push(issue(
                "native_audit_physical_scope_invalid",
                a,
                ids,
                o["asset"].clone(),
                Value::Null,
                json!({"chain":k.chain,"network":k.network,"native_namespace":k.native_namespace}),
            )?);
            continue;
        }
        let asset = text(&a["asset"]).py_upper();
        let from = text(&a["from_wallet_id"]);
        let wallet = text(&a["to_wallet_id"]);
        let r = exact_msat(&a["crypto_received_msat"]);
        let f = exact_msat(&a["crypto_fee_msat"]);
        let sent = exact_msat(&a["crypto_sent_msat"]);
        if out.is_empty()
            || input.is_empty()
            || wallet.is_empty()
            || from != o["wallet_id"]
            || asset != o["asset"]
            || r.as_ref().is_none_or(|n| *n <= BigInt::from(0))
            || f.is_none()
            || sent.is_none()
            || sent != r.as_ref().zip(f.as_ref()).map(|(r, f)| r + f)
            || text(&a["occurred_at"]) != o["occurred_at"]
        {
            issues.push(issue("native_audit_leg_invalid",a,ids,o["asset"].clone(),r.as_ref().map(number).unwrap_or(Value::Null),json!({"source_wallet_id":o["wallet_id"],"audit_from_wallet_id":from,"target_wallet_id":wallet,"source_asset":o["asset"],"audit_asset":asset,"exact_msat_fields_present":r.is_some()&&f.is_some()&&sent.is_some()}))?);
            continue;
        }
        let received = r.unwrap();
        let fee = f.unwrap();
        let evidence = hash(
            &json!({"schema_version":1,"kind":"verified_native_intra_leg","pairing_source":codec::pystr(&a["pairing_source"]),"source_quantity_hash":o["quantity_hash"],"out_id":out,"in_id":input,"target_wallet_id":wallet,"asset":asset,"received_msat":number(&received),"fee_msat":number(&fee)}),
        )?;
        legs.push(Leg {
            evidence,
            out,
            input,
            source_id,
            source: Value::clone(o),
            wallet,
            asset,
            received,
            fee,
            pairing,
            time: text(&o["occurred_at"]),
        });
    }
    Ok(legs)
}
fn reserved(v: &Value, hash: &str) -> Result<BigInt, String> {
    let value = v.get(hash).cloned().unwrap_or(json!(0));
    if value.is_null() || value.is_array() || value.is_object() {
        return Err("TypeError".into());
    }
    let value = codec::integer(&value).map_err(|_| "ValueError".to_owned())?;
    exact(&value)
}

type Slot = (EventKey, String, String);
fn targets(
    events: &mut BTreeMap<EventKey, Cow<'_, Value>>,
    legs: &[Leg],
    issues: &mut Vec<Value>,
) -> Result<(BTreeMap<String, String>, BTreeSet<String>), String> {
    let mut slots: BTreeMap<Slot, Vec<Leg>> = BTreeMap::new();
    for l in legs {
        slots
            .entry((
                key(&l.source["event_key"]),
                l.wallet.clone(),
                l.asset.clone(),
            ))
            .or_default()
            .push(l.clone());
    }
    for members in slots.values_mut() {
        members.sort_by_cached_key(|l| l.evidence.clone());
    }
    let mut invalid = BTreeSet::new();
    let mut owners: BTreeMap<EventKey, BTreeMap<String, Option<Slot>>> = events
        .iter()
        .map(|(k, e)| {
            (
                k.clone(),
                aliases(e).into_keys().map(|id| (id, None)).collect(),
            )
        })
        .collect();
    for (slot, members) in &slots {
        let (k, wallet, asset) = slot;
        let event = events.get(k).ok_or("KeyError")?;
        let amount: BigInt = members.iter().map(|l| &l.received).sum();
        let existing: Vec<_> = array(&event["legs"])
            .iter()
            .filter(|o| {
                o["direction"] == "inbound" && o["wallet_id"] == *wallet && o["asset"] == *asset
            })
            .collect();
        let audit = json!({"pairing_source":members[0].pairing,"occurred_at":members[0].time});
        let ids = members
            .iter()
            .flat_map(|l| [l.source_id.clone(), l.input.clone()])
            .collect();
        if !existing.is_empty() && (existing.len() != 1 || principal(existing[0])? != amount) {
            invalid.insert(slot.clone());
            issues.push(issue("native_audit_target_contradiction",&audit,ids,json!(asset),number(&amount),json!({"wallet_id":wallet,"observed_target_amounts_msat":existing.iter().map(|o|principal(*o).map(|n|number(&n))).collect::<Result<Vec<_>,_>>()?,"chain":k.chain,"network":k.network}))?);
            continue;
        }
        if !existing.is_empty() {
            continue;
        }
        let owner = owners.get_mut(k).ok_or("KeyError")?;
        let original = aliases(event);
        let mut conflicts = Vec::new();
        for l in members {
            if let Some(prior) = owner.get(&l.input) {
                if prior.as_ref() == Some(slot) {
                    continue;
                }
                let mut detail = json!({"transaction_id":l.input});
                if let Some(prior) = prior {
                    detail["target_wallet_id"] = json!(prior.1);
                    detail["asset"] = json!(prior.2);
                    invalid.insert(prior.clone());
                } else {
                    detail["quantity_hash"] = original.get(&l.input).ok_or("KeyError")?.clone();
                }
                conflicts.push(detail);
            }
        }
        if !conflicts.is_empty() {
            invalid.insert(slot.clone());
            issues.push(issue("native_audit_alias_contradiction",&audit,ids,json!(asset),number(&amount),json!({"wallet_id":wallet,"conflicting_aliases":conflicts,"chain":k.chain,"network":k.network}))?);
            continue;
        }
        for l in members {
            owner.insert(l.input.clone(), Some(slot.clone()));
        }
    }
    let mut invalid_sources: BTreeSet<_> = slots
        .iter()
        .filter(|(slot, _)| invalid.contains(*slot))
        .flat_map(|(_, ls)| ls.iter().map(Leg::source_hash))
        .collect();
    loop {
        let mut changed = false;
        for (slot, members) in &slots {
            if !invalid.contains(slot)
                && members
                    .iter()
                    .any(|l| invalid_sources.contains(&l.source_hash()))
            {
                invalid.insert(slot.clone());
                invalid_sources.extend(members.iter().map(Leg::source_hash));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let rejected: BTreeSet<_> = slots
        .iter()
        .filter(|(slot, _)| invalid.contains(*slot))
        .flat_map(|(_, ls)| ls.iter().map(|l| l.evidence.clone()))
        .collect();
    let mut result = BTreeMap::new();
    for (slot, members) in slots {
        if invalid.contains(&slot) {
            continue;
        }
        let (k, wallet, asset) = slot;
        let event = events.get_mut(&k).ok_or("KeyError")?.to_mut();
        let amount: BigInt = members.iter().map(|l| &l.received).sum();
        let existing = array(&event["legs"])
            .iter()
            .find(|o| {
                o["direction"] == "inbound" && o["wallet_id"] == wallet && o["asset"] == asset
            })
            .cloned();
        let target = if let Some(o) = existing {
            o
        } else {
            let proof = json!({"schema_version":1,"kind":"verified_native_owned_destination","event":k.payload(),"wallet_id":wallet,"asset":asset,"amount_msat":number(&amount),"legs":members.iter().map(|l|json!({"evidence_id":l.evidence,"pairing_source":l.pairing,"source_quantity_hash":l.source["quantity_hash"],"received_msat":number(&l.received)})).collect::<Vec<_>>()});
            let proof_hash = hash(&proof)?;
            let o = synthetic(
                json!({"id":format!("native-owned-in:{proof_hash}"),"journal_transaction_id":members[0].source_id,"wallet_id":wallet,"direction":"inbound","asset":asset,"amount":number(&amount),"fee":0,"amount_includes_fee":false,"occurred_at":members.iter().map(|l|&l.time).min().unwrap(),"fingerprint":format!("native-owned-in:{proof_hash}"),"kind":"self_transfer_in","raw_json":{"txid":k.native_event_id,"_kassiber_quantity_evidence":proof}}),
                &k,
            )?;
            let mut aliases = aliases(event);
            for l in &members {
                aliases.insert(l.input.clone(), o["quantity_hash"].clone());
            }
            add(event, o.clone(), aliases);
            o
        };
        for l in members {
            result.insert(l.evidence, text(&target["quantity_hash"]));
        }
    }
    Ok((result, rejected))
}
fn claim(
    id: String,
    source: Value,
    target: Value,
    state: &str,
    reason: &str,
    bundle: &str,
) -> Value {
    json!({"$type":"QuantityClaim","claim_id":id,"source":source,"target":target,"state":state,"priority":{"$enum":"ClaimPriority.EXACT_NATIVE_EVENT","value":20},"reason":reason,"atomic_bundle_id":bundle,"destination_kind":null,"external_economic_subtype":null,"transfer_kind":null,"transfer_policy":null,"component_id":null,"allow_cross_rail":false})
}
fn slice(hash: &str, start: &BigInt, end: &BigInt) -> Value {
    json!({"$type":"QuantitySlice","observation_hash":hash,"start_msat":number(start),"end_msat":number(end)})
}
/// Return augmented canonical input and claims, without granting storage authority.
pub fn compile(
    canonical: &Value,
    audits: &[Value],
    excluded: &[Value],
    reservation: &Value,
) -> Result<Value, String> {
    let compilation = compile_borrowed(canonical, audits, excluded, reservation, None)?;
    Ok(
        json!({"$type":"NativeAuditCompilation","canonical_input":compilation.input(),"claims":compilation.claims,"issues":compilation.issues}),
    )
}

pub(super) struct Compilation<'a> {
    pub events: Vec<super::QEvent<'a>>,
    pub rejected: &'a [Value],
    pub claims: Vec<Value>,
    pub issues: Vec<Value>,
}
impl Compilation<'_> {
    pub fn input(&self) -> Value {
        json!({"$type":"CanonicalQuantityInput","events":self.events.iter().map(|v|v.get().value().into_owned()).collect::<Vec<_>>(),"rejected_events":self.rejected})
    }
}

pub(super) fn compile_borrowed<'a>(
    canonical: &'a Value,
    audits: &[Value],
    excluded: &[Value],
    reservation: &Value,
    shared: Option<&[(String, LegRef<'_>)]>,
) -> Result<Compilation<'a>, String> {
    let original = array(&canonical["events"]);
    let event_key = |event: &'a Value| {
        let key = &event["event_key"];
        ["chain", "network", "native_namespace", "native_event_id"]
            .map(|field| super::text_key(&key[field]))
    };
    if audits.is_empty()
        && shared.is_some()
        && original
            .windows(2)
            .all(|pair| event_key(&pair[0]) < event_key(&pair[1]))
    {
        return Ok(Compilation {
            events: original
                .iter()
                .map(|e| super::QEvent::Ref(EventRef::Json(e)))
                .collect(),
            rejected: array(&canonical["rejected_events"]),
            claims: vec![],
            issues: vec![],
        });
    }
    let mut events: BTreeMap<_, _> = array(&canonical["events"])
        .iter()
        .map(|e| (key(&e["event_key"]), Cow::Borrowed(e)))
        .collect();
    let mut issues = Vec::new();
    virtual_sources(&mut events, audits, &mut issues)?;
    let excluded = excluded
        .iter()
        .filter(|v| codec::truth(v))
        .map(codec::pystr)
        .collect();
    let original = array(&canonical["events"]);
    let shared = shared.filter(|_| {
        events.len() == original.len()
            && events
                .values()
                .zip(original)
                .all(|(event, prior)| matches!(event, Cow::Borrowed(e) if std::ptr::eq(*e, prior)))
    });
    let legs = parse(&events, audits, &excluded, &mut issues, shared)?;
    let mut groups: BTreeMap<String, Vec<Leg>> = BTreeMap::new();
    for l in legs {
        groups.entry(l.source_hash()).or_default().push(l);
    }
    let mut valid = Vec::new();
    for (hash, mut members) in groups {
        members.sort_by_cached_key(|l| l.evidence.clone());
        let src = &members[0].source;
        let received: BigInt = members.iter().map(|l| &l.received).sum();
        let fee: BigInt = members.iter().map(|l| &l.fee).sum();
        let res = reserved(reservation, &hash)?;
        let cap = principal(src)?;
        let residual = &cap - &res - &received;
        if res >= BigInt::from(0)
            && res <= cap
            && residual >= BigInt::from(0)
            && fee == exact(&src["fee_msat"])?
            && (residual == BigInt::from(0)
                || members.iter().all(|l| l.pairing == "ownership_derived"))
        {
            valid.extend(members);
            continue;
        }
        let ids = std::iter::once(text(&src["transaction_id"]))
            .chain(members.iter().map(|l| l.out.clone()))
            .chain(members.iter().map(|l| l.input.clone()))
            .collect();
        issues.push(issue("native_audit_source_conservation_invalid",&json!({"pairing_source":members[0].pairing,"occurred_at":src["occurred_at"]}),ids,src["asset"].clone(),number(&cap),json!({"source_quantity_hash":hash,"source_principal_msat":number(&cap),"source_fee_msat":src["fee_msat"],"received_msat":number(&received),"audited_fee_msat":number(&fee),"reserved_source_msat":number(&res),"residual_msat":number(&residual)}))?);
    }
    let (targets, invalid) = targets(&mut events, &valid, &mut issues)?;
    let invalid_sources: BTreeSet<_> = valid
        .iter()
        .filter(|l| invalid.contains(&l.evidence))
        .map(Leg::source_hash)
        .collect();
    let mut groups: BTreeMap<String, Vec<Leg>> = BTreeMap::new();
    for l in valid {
        if !invalid.contains(&l.evidence) && !invalid_sources.contains(&l.source_hash()) {
            groups.entry(l.source_hash()).or_default().push(l);
        }
    }
    let mut claims = Vec::new();
    let mut target_cursors: BTreeMap<String, BigInt> = BTreeMap::new();
    for (hash, mut members) in groups {
        members.sort_by_cached_key(|l| l.evidence.clone());
        let source = members[0].source.clone();
        let bundle = format!("engine-native:{hash}");
        let mut cursor = BigInt::from(0);
        for l in members {
            let target = targets.get(&l.evidence).ok_or("KeyError")?;
            let start = target_cursors.get(target).cloned().unwrap_or_default();
            let end = &start + &l.received;
            let source_end = &cursor + &l.received;
            claims.push(claim(
                format!("engine-native:{}:retained", l.evidence),
                slice(&hash, &cursor, &source_end),
                slice(target, &start, &end),
                "internal_verified",
                &l.pairing,
                &bundle,
            ));
            cursor = source_end;
            target_cursors.insert(target.clone(), end);
        }
        let available = principal(&source)? - reserved(reservation, &hash)?;
        if source["fee_attribution"] == "implicit_wallet_delta" && cursor < available {
            claims.push(claim(
                format!("engine-native:{hash}:implicit-residual"),
                slice(&hash, &cursor, &available),
                Value::Null,
                "custody_suspense",
                "implicit_wallet_delta_unallocated",
                &bundle,
            ));
        }
    }
    claims.sort_by_cached_key(|c| text(&c["claim_id"]));
    issues.sort_by_cached_key(|i| text(&i["issue_id"]));
    Ok(Compilation {
        events: events
            .into_values()
            .map(|e| match e {
                Cow::Borrowed(e) => super::QEvent::Ref(EventRef::Json(e)),
                Cow::Owned(e) => super::QEvent::Owned(e),
            })
            .collect(),
        rejected: array(&canonical["rejected_events"]),
        claims,
        issues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantity_native_borrowed_event_order() {
        let cases: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_quantity_composition.json"
        ))
        .unwrap();
        let canonical = cases
            .as_array()
            .unwrap()
            .iter()
            .map(|c| &c["canonical"])
            .find(|c| array(&c["events"]).len() > 1)
            .unwrap();
        let expected = compile(canonical, &[], &[], &json!({})).unwrap();
        let mut reversed = canonical.clone();
        reversed["events"].as_array_mut().unwrap().reverse();
        let actual = compile_borrowed(&reversed, &[], &[], &json!({}), Some(&[])).unwrap();
        assert_eq!(actual.input(), expected["canonical_input"]);
        let mut duplicate = canonical.clone();
        let mut last = duplicate["events"][0].clone();
        last["evidence_snapshots"] = json!([]);
        duplicate["events"].as_array_mut().unwrap().push(last);
        let expected = compile(&duplicate, &[], &[], &json!({})).unwrap();
        let actual = compile_borrowed(&duplicate, &[], &[], &json!({}), Some(&[])).unwrap();
        assert_eq!(actual.input(), expected["canonical_input"]);
    }
}
