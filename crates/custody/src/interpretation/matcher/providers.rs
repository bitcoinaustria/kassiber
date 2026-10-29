//! Provider edges are independent conflicts even when another method is exact.
use super::*;
use kassiber_pyjson::Value as Py;

#[derive(Clone)]
struct Evidence {
    provider: String,
    id: String,
    kind: Option<String>,
    flow: String,
    status: String,
    send: String,
    receive: String,
    send_amount: Option<BigInt>,
    receive_amount: Option<BigInt>,
    amount_conflict: bool,
    route_conflict: bool,
    identity_conflict: bool,
    semantic_conflict: bool,
}
// Lossless private encoding for Python surrogate identities. Escape the reserved
// scalar range too, so a valid astral identity cannot collide with a surrogate.
// Only candidate topology consumes these strings; they never leave this module.
fn identity_text(v: &Py) -> String {
    if let Py::Str(value) = v {
        let mut result = String::new();
        for point in value.code_points() {
            if (0xd800..=0xdfff).contains(&point) {
                result.push(char::from_u32(0xf0000 + point - 0xd800).unwrap());
            } else {
                if (0xf0000..=0xf0800).contains(&point) {
                    result.push('\u{f0800}');
                }
                result.push(char::from_u32(point).unwrap());
            }
        }
        result
    } else {
        super::super::text::string(v)
    }
}
fn all(payload: &Py, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter_map(|key| {
            let v = s::get(payload, key);
            if matches!(v, Py::Null) {
                None
            } else {
                let text = identity_text(v).py_strip().to_owned();
                (!text.is_empty()).then_some(text)
            }
        })
        .collect()
}
fn first(payload: &Py, keys: &[&str]) -> String {
    all(payload, keys).into_iter().next().unwrap_or_default()
}
fn flow(v: &str) -> String {
    v.py_strip().py_lower().replace(['_', ' '], "-")
}
fn kind(v: &str) -> Option<String> {
    Some(
        match flow(v).as_str() {
            "chain" | "chain-swap" | "chainswap" => "chain-swap",
            "reverse" | "reverse-swap" | "reverse-submarine" | "reverse-submarine-swap" => {
                "reverse-submarine-swap"
            }
            "submarine" | "submarine-swap" => "submarine-swap",
            "refund" | "swap-refund" | "failed-swap-refund" => "swap-refund",
            _ => return None,
        }
        .into(),
    )
}
fn count(values: impl IntoIterator<Item = String>) -> usize {
    values.into_iter().collect::<BTreeSet<_>>().len()
}
// CPython 3.11 Unicode 14 digit-only additions to the shared decimal table.
fn is_digit(c: char) -> bool {
    crate::python_text::decimal_digit(c).is_some()
        || matches!(c, '\u{b2}'..='\u{b3}' | '\u{b9}' | '\u{1369}'..='\u{1371}' | '\u{19da}' | '\u{2070}' | '\u{2074}'..='\u{2079}' | '\u{2080}'..='\u{2089}' | '\u{2460}'..='\u{2468}' | '\u{2474}'..='\u{247c}' | '\u{2488}'..='\u{2490}' | '\u{24ea}' | '\u{24f5}'..='\u{24fd}' | '\u{24ff}' | '\u{2776}'..='\u{277e}' | '\u{2780}'..='\u{2788}' | '\u{278a}'..='\u{2792}' | '\u{10a40}'..='\u{10a43}' | '\u{10e60}'..='\u{10e68}' | '\u{11052}'..='\u{1105a}' | '\u{1f100}'..='\u{1f10a}')
}
fn integer(payload: &Py, key: &str) -> Result<Option<BigInt>, String> {
    let value = s::get(payload, key);
    let value = match value {
        Py::Int(n) => Some(n.clone()),
        Py::Str(v) => {
            let Some(v) = v.as_str() else {
                return Ok(None);
            };
            let v = v.py_strip();
            if !v.is_empty() && v.chars().all(is_digit) {
                Some(s::integer(value).map_err(str::to_owned)?)
            } else {
                None
            }
        }
        _ => None,
    };
    Ok(value.filter(|n| *n >= BigInt::from(0)))
}
fn evidence(row: &Value) -> Result<Option<Evidence>, String> {
    let payload = htlc::raw(row);
    let providers: Vec<_> = all(&payload, &["provider", "source", "source_format"])
        .into_iter()
        .map(|v| {
            let v = v.py_strip().py_lower().replace('-', "_");
            if v.contains("bullbitcoin") || v == "bull" {
                "bullbitcoin".into()
            } else if v.contains("boltz") {
                "boltz".into()
            } else {
                v
            }
        })
        .filter(|v| !v.is_empty())
        .collect();
    let Some(provider) = providers.first() else {
        return Ok(None);
    };
    let ids = all(
        &payload,
        &[
            "swap_id",
            "swapId",
            "swapID",
            "provider_swap_id",
            "boltz_id",
            "boltzId",
        ],
    );
    let id = ids.first().cloned().unwrap_or_else(|| {
        if matches!(provider.as_str(), "boltz" | "bullbitcoin") {
            first(&payload, &["id"])
        } else {
            String::new()
        }
    });
    if id.is_empty() {
        return Ok(None);
    }
    let flows = all(&payload, &["flow", "type", "swap_type", "swapType", "kind"]);
    let statuses = all(&payload, &["status", "state", "finality"]);
    let f = flows.first().cloned().unwrap_or_default();
    let status = statuses.first().cloned().unwrap_or_default();
    let kind = if flow(&status).contains("refund") {
        Some("swap-refund".into())
    } else {
        kind(&f)
    };
    let sends = all(
        &payload,
        &["send_txid", "sendTxid", "lockup_txid", "lockupTxid"],
    );
    let receives = all(
        &payload,
        &[
            "receive_txid",
            "receiveTxid",
            "claim_txid",
            "claimTxid",
            "refund_txid",
            "refundTxid",
        ],
    );
    let send = sends.first().cloned().unwrap_or_default();
    let receive = receives.first().cloned().unwrap_or_default();
    if kind.is_none() && send.is_empty() && receive.is_empty() {
        return Ok(None);
    }
    Ok(Some(Evidence {
        provider: provider.clone(),
        id,
        kind,
        flow: flow(&f),
        status,
        send,
        receive,
        send_amount: integer(&payload, "send_amount_msat")?,
        receive_amount: integer(&payload, "receive_amount_msat")?,
        amount_conflict: false,
        route_conflict: count(sends.iter().map(|v| v.py_lower())) > 1
            || count(receives.iter().map(|v| v.py_lower())) > 1,
        identity_conflict: count(providers) > 1 || count(ids.iter().map(|v| v.py_lower())) > 1,
        semantic_conflict: count(flows.iter().map(|v| flow(v))) > 1
            || count(statuses.iter().map(|v| flow(v))) > 1,
    }))
}
fn merge(a: &Evidence, b: &Evidence) -> Evidence {
    let select = |a: &str, b: &str| {
        if a.is_empty() {
            b.to_owned()
        } else {
            a.to_owned()
        }
    };
    let conflict = |a: &str, b: &str| {
        !a.is_empty() && !b.is_empty() && a.py_strip().py_lower() != b.py_strip().py_lower()
    };
    Evidence {
        provider: a.provider.clone(),
        id: a.id.clone(),
        kind: if a.kind.as_deref() == Some("swap-refund")
            || b.kind.as_deref() == Some("swap-refund")
        {
            Some("swap-refund".into())
        } else {
            a.kind.clone().or_else(|| b.kind.clone())
        },
        flow: select(&a.flow, &b.flow),
        status: select(&a.status, &b.status),
        send: select(&a.send, &b.send),
        receive: select(&a.receive, &b.receive),
        send_amount: a.send_amount.clone().or_else(|| b.send_amount.clone()),
        receive_amount: a
            .receive_amount
            .clone()
            .or_else(|| b.receive_amount.clone()),
        amount_conflict: a.amount_conflict
            || b.amount_conflict
            || a.send_amount.is_some() && b.send_amount.is_some() && a.send_amount != b.send_amount
            || a.receive_amount.is_some()
                && b.receive_amount.is_some()
                && a.receive_amount != b.receive_amount,
        route_conflict: a.route_conflict
            || b.route_conflict
            || conflict(&a.send, &b.send)
            || conflict(&a.receive, &b.receive),
        identity_conflict: a.identity_conflict
            || b.identity_conflict
            || a.provider != b.provider
            || a.id.py_lower() != b.id.py_lower(),
        semantic_conflict: a.semantic_conflict
            || b.semantic_conflict
            || !a.flow.is_empty() && !b.flow.is_empty() && flow(&a.flow) != flow(&b.flow)
            || !a.status.is_empty() && !b.status.is_empty() && flow(&a.status) != flow(&b.status),
    }
}
fn route(expected: &str, row: &Value) -> bool {
    let id = codec::text(&row["external_id"]);
    let id = id.py_strip();
    id.is_empty() || expected.is_empty() || id.py_lower() == expected.py_strip().py_lower()
}
fn whole(e: &Evidence, out: &Value, incoming: &Value) -> Result<bool, String> {
    let Some(send) = graph::strict_txid(&json!(e.send)) else {
        return Ok(false);
    };
    let Some(receive) = graph::strict_txid(&json!(e.receive)) else {
        return Ok(false);
    };
    let Some((a, b)) = graph::onchain_scope(out).zip(graph::onchain_scope(incoming)) else {
        return Ok(false);
    };
    if a.2 != send || b.2 != receive {
        return Ok(false);
    }
    let (a, b) = (
        recorded::network_domain(out),
        recorded::network_domain(incoming),
    );
    if a.is_none()
        || b.is_none()
        || a != b
        || e.amount_conflict
        || e.route_conflict
        || e.identity_conflict
        || e.semantic_conflict
    {
        return Ok(false);
    }
    if e.send_amount != Some(amount(out)?) || e.receive_amount != Some(amount(incoming)?) {
        return Ok(false);
    }
    conserves(out, incoming)
}
type Edge<'a> = (&'a Value, &'a Value, bool, Option<String>);
pub(super) fn edges<'a>(outs: &[&'a Value], ins: &[&'a Value]) -> Result<Vec<Edge<'a>>, String> {
    let outs = outs
        .iter()
        .map(|r| evidence(r).map(|e| e.map(|e| (*r, e))))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let ins = ins
        .iter()
        .map(|r| evidence(r).map(|e| e.map(|e| (*r, e))))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let mut keys = Vec::new();
    for (_, e) in &outs {
        let k = (&e.provider, &e.id);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let mut result = Vec::new();
    for key in keys {
        let sources: Vec<_> = outs
            .iter()
            .filter(|(_, e)| (&e.provider, &e.id) == key)
            .collect();
        let targets: Vec<_> = ins
            .iter()
            .filter(|(_, e)| (&e.provider, &e.id) == key)
            .collect();
        for (out, a) in &sources {
            for (incoming, b) in &targets {
                if !compatible(out, incoming) {
                    continue;
                }
                let unique = sources
                    .iter()
                    .filter(|(r, _)| compatible(r, incoming))
                    .count()
                    == 1
                    && targets.iter().filter(|(r, _)| compatible(out, r)).count() == 1;
                let e = merge(a, b);
                if !route(&e.send, out) || !route(&e.receive, incoming) {
                    continue;
                }
                let exact = unique && whole(&e, out, incoming)?;
                result.push((*out, *incoming, exact, e.kind));
            }
        }
    }
    Ok(result)
}
