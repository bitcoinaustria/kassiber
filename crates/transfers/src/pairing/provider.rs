use super::*;
use matcher::{conserves, Row};
#[derive(Clone, Default)]
pub(super) struct Evidence {
    pub provider: String,
    pub id: String,
    pub kind: String,
    pub flow: String,
    pub status: String,
    pub send_txid: String,
    pub receive_txid: String,
    pub version: String,
    pub taproot: String,
    pub cooperative: String,
    pub spend_path: String,
    pub send_amount: Option<i128>,
    pub receive_amount: Option<i128>,
    pub amount_conflict: bool,
    pub route_conflict: bool,
    pub identity_conflict: bool,
    pub semantic_conflict: bool,
}
fn all(p: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter_map(|k| get(p, k).filter(|v| !matches!(v, Value::Null)))
        .map(|v| kassiber_pytext::strip(&py_str(v)).to_owned())
        .filter(|v| !v.is_empty())
        .collect()
}
fn first(p: &Value, keys: &[&str]) -> String {
    all(p, keys).into_iter().next().unwrap_or_default()
}
fn normalized_provider(v: &str) -> String {
    let v = lower(v).replace('-', "_");
    if v.contains("bullbitcoin") || v == "bull" {
        "bullbitcoin".into()
    } else if v.contains("boltz") {
        "boltz".into()
    } else {
        v
    }
}
fn flow(v: &str) -> String {
    lower(v).replace(['_', ' '], "-")
}
fn kind(v: &str) -> String {
    match flow(v).as_str() {
        "chain" | "chain-swap" | "chainswap" => "chain-swap",
        "reverse" | "reverse-swap" | "reverse-submarine" | "reverse-submarine-swap" => {
            "reverse-submarine-swap"
        }
        "submarine" | "submarine-swap" => "submarine-swap",
        "refund" | "swap-refund" | "failed-swap-refund" => "swap-refund",
        _ => "",
    }
    .into()
}
fn amount(p: &Value, k: &str) -> Result<Option<i128>> {
    let Some(v) = get(p, k) else { return Ok(None) };
    let amount = match v {
        Value::Int(_) => n(v)?,
        Value::Str(v) => {
            let Some(v) = v.as_str() else { return Ok(None) };
            // `_first_nonnegative_int`: string amounts use ASCII decimal
            // digits; any other digit is no evidence (b654fad74).
            let v = kassiber_pytext::strip(v);
            if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                return Ok(None);
            }
            // `int()` refuses past CPython's integer-string limit; the
            // reference skips such an amount as it skips any ValueError.
            if v.len() > kassiber_pyjson::INT_MAX_STR_DIGITS {
                return Ok(None);
            }
            n(&s(v))?
        }
        _ => return Ok(None),
    };
    Ok((amount >= 0).then_some(amount))
}
fn conflict(values: impl IntoIterator<Item = String>) -> bool {
    values.into_iter().collect::<HashSet<_>>().len() > 1
}
pub(super) fn extract(p: &Value) -> Result<Option<Evidence>> {
    let providers: Vec<_> = all(p, &["provider", "source", "source_format"])
        .iter()
        .map(|v| normalized_provider(v))
        .filter(|v| !v.is_empty())
        .collect();
    let Some(provider) = providers.first().cloned() else {
        return Ok(None);
    };
    let ids = all(
        p,
        &[
            "swap_id",
            "swapId",
            "swapID",
            "provider_swap_id",
            "boltz_id",
            "boltzId",
        ],
    );
    let mut id = ids.first().cloned().unwrap_or_default();
    if id.is_empty() && ["boltz", "bullbitcoin"].contains(&provider.as_str()) {
        id = first(p, &["id"]);
    }
    if id.is_empty() {
        return Ok(None);
    }
    let flows = all(p, &["flow", "type", "swap_type", "swapType", "kind"]);
    let statuses = all(p, &["status", "state", "finality"]);
    let f = flows.first().cloned().unwrap_or_default();
    let status = statuses.first().cloned().unwrap_or_default();
    let kind = if flow(&status).contains("refund") {
        "swap-refund".into()
    } else {
        kind(&f)
    };
    let sends = all(p, &["send_txid", "sendTxid", "lockup_txid", "lockupTxid"]);
    let receives = all(
        p,
        &[
            "receive_txid",
            "receiveTxid",
            "claim_txid",
            "claimTxid",
            "refund_txid",
            "refundTxid",
        ],
    );
    let send_txid = sends.first().cloned().unwrap_or_default();
    let receive_txid = receives.first().cloned().unwrap_or_default();
    if kind.is_empty() && send_txid.is_empty() && receive_txid.is_empty() {
        return Ok(None);
    }
    Ok(Some(Evidence {
        provider,
        id,
        kind,
        flow: flow(&f),
        status,
        send_txid,
        receive_txid,
        version: first(p, &["version", "swap_version", "swapVersion"]),
        taproot: first(p, &["taproot", "is_taproot", "isTaproot"]),
        cooperative: first(p, &["cooperative", "cooperative_claim", "cooperativeClaim"]),
        spend_path: first(p, &["spend_path", "spendPath", "claim_path", "claimPath"]),
        send_amount: amount(p, "send_amount_msat")?,
        receive_amount: amount(p, "receive_amount_msat")?,
        route_conflict: conflict(sends.iter().map(|v| kassiber_pytext::lower(v)))
            || conflict(receives.iter().map(|v| kassiber_pytext::lower(v))),
        identity_conflict: conflict(providers)
            || conflict(ids.iter().map(|v| kassiber_pytext::lower(v))),
        semantic_conflict: conflict(flows.iter().map(|v| flow(v)))
            || conflict(statuses.iter().map(|v| flow(v))),
        ..Evidence::default()
    }))
}
// CPython sets start at eight slots. Merging two integer amount claims retains
// the first table slot (not input order), including a collision's linear probe.
// This matters for the surfaced amount on contradictory provider metadata.
fn set_first(a: Option<i128>, b: Option<i128>) -> Option<i128> {
    match (a, b) {
        (Some(a), Some(b)) if a != b => {
            let hash = |v: i128| v.rem_euclid((1_i128 << 61) - 1) as u64;
            let first = hash(a) & 7;
            let mut second = hash(b) & 7;
            let mut perturb = hash(b);
            while second == first {
                perturb >>= 5;
                second = (second * 5 + 1 + perturb) & 7;
            }
            Some(if second < first { b } else { a })
        }
        (Some(a), _) => Some(a),
        (_, b) => b,
    }
}
fn two_conflict(a: Option<i128>, b: Option<i128>) -> bool {
    a.zip(b).is_some_and(|(a, b)| a != b)
}
fn text_conflict(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && lower(a) != lower(b)
}
fn merge(a: &Evidence, b: &Evidence) -> Evidence {
    let or = |a: &String, b: &String| if a.is_empty() { b.clone() } else { a.clone() };
    Evidence {
        provider: a.provider.clone(),
        id: a.id.clone(),
        kind: if a.kind == "swap-refund" || b.kind == "swap-refund" {
            "swap-refund".into()
        } else {
            or(&a.kind, &b.kind)
        },
        flow: or(&a.flow, &b.flow),
        status: or(&a.status, &b.status),
        send_txid: or(&a.send_txid, &b.send_txid),
        receive_txid: or(&a.receive_txid, &b.receive_txid),
        version: or(&a.version, &b.version),
        taproot: or(&a.taproot, &b.taproot),
        cooperative: or(&a.cooperative, &b.cooperative),
        spend_path: or(&a.spend_path, &b.spend_path),
        send_amount: set_first(a.send_amount, b.send_amount),
        receive_amount: set_first(a.receive_amount, b.receive_amount),
        amount_conflict: a.amount_conflict
            || b.amount_conflict
            || two_conflict(a.send_amount, b.send_amount)
            || two_conflict(a.receive_amount, b.receive_amount),
        route_conflict: a.route_conflict
            || b.route_conflict
            || text_conflict(&a.send_txid, &b.send_txid)
            || text_conflict(&a.receive_txid, &b.receive_txid),
        identity_conflict: a.identity_conflict
            || b.identity_conflict
            || a.provider != b.provider
            || kassiber_pytext::lower(&a.id) != kassiber_pytext::lower(&b.id),
        semantic_conflict: a.semantic_conflict
            || b.semantic_conflict
            || (!a.flow.is_empty() && !b.flow.is_empty() && flow(&a.flow) != flow(&b.flow))
            || (!a.status.is_empty() && !b.status.is_empty() && flow(&a.status) != flow(&b.status)),
    }
}
fn route(e: &Evidence, r: &Row, out: bool) -> bool {
    let id = kassiber_pytext::strip(&st(&r.data, "external_id")).to_owned();
    let expected = if out { &e.send_txid } else { &e.receive_txid };
    id.is_empty() || expected.is_empty() || kassiber_pytext::lower(&id) == lower(expected)
}
fn whole(e: &Evidence, o: &Row, i: &Row) -> bool {
    let (Some(send), Some(receive), Some(os), Some(ins)) = (
        hex_id(&e.send_txid),
        hex_id(&e.receive_txid),
        o.scope.as_ref(),
        i.scope.as_ref(),
    ) else {
        return false;
    };
    os.2 == send
        && ins.2 == receive
        && o.domain.is_some()
        && o.domain == i.domain
        && !e.amount_conflict
        && !e.route_conflict
        && !e.identity_conflict
        && !e.semantic_conflict
        && e.send_amount == Some(o.amount)
        && e.receive_amount == Some(i.amount)
        && conserves(o, i)
}
pub(super) fn pairs(
    rows: &[Row],
    outs: &[usize],
    ins: &[usize],
) -> Result<Vec<(usize, usize, Evidence, bool)>> {
    // A vector keeps insertion order; the index only accelerates key lookup.
    let mut groups: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    let mut index = HashMap::new();
    for (side, indices) in [outs, ins].iter().enumerate() {
        for &idx in *indices {
            if let Some(e) = rows[idx].provider.as_ref().map_err(Clone::clone)? {
                let key = (&e.provider, &e.id);
                let pos = *index.entry(key).or_insert_with(|| {
                    groups.push((Vec::new(), Vec::new()));
                    groups.len() - 1
                });
                if side == 0 {
                    groups[pos].0.push(idx)
                } else {
                    groups[pos].1.push(idx)
                }
            }
        }
    }
    let mut result = Vec::new();
    for (outs, ins) in groups {
        let mut oc = HashMap::new();
        let mut ic = HashMap::new();
        for &o in &outs {
            *oc.entry(rows[o].domain.clone()).or_insert(0usize) += 1;
        }
        for &i in &ins {
            *ic.entry(rows[i].domain.clone()).or_insert(0usize) += 1;
        }
        for &oi in &outs {
            let o = &rows[oi];
            for &ii in &ins {
                let i = &rows[ii];
                if !compatible(&o.domain, &i.domain) {
                    continue;
                }
                let compatible_outs = if i.domain.is_none() {
                    outs.len()
                } else {
                    oc.get(&i.domain).copied().unwrap_or(0) + oc.get(&None).copied().unwrap_or(0)
                };
                let compatible_ins = if o.domain.is_none() {
                    ins.len()
                } else {
                    ic.get(&o.domain).copied().unwrap_or(0) + ic.get(&None).copied().unwrap_or(0)
                };
                let e = merge(
                    o.provider.as_ref().unwrap().as_ref().unwrap(),
                    i.provider.as_ref().unwrap().as_ref().unwrap(),
                );
                if !route(&e, o, true) || !route(&e, i, false) {
                    continue;
                }
                let exact = compatible_outs == 1 && compatible_ins == 1 && whole(&e, o, i);
                result.push((oi, ii, e, exact));
            }
        }
    }
    Ok(result)
}
