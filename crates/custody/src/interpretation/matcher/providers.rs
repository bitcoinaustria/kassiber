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
fn integer(payload: &Py, key: &str) -> Result<Option<BigInt>, String> {
    let value = s::get(payload, key);
    let value = match value {
        Py::Int(n) => Some(n.clone()),
        Py::Str(v) => {
            let Some(v) = v.as_str() else {
                return Ok(None);
            };
            // `_first_nonnegative_int` (b654fad74): ASCII decimal digits only,
            // and a string past CPython's 4300-digit int limit is skipped like
            // the ValueError Python catches.
            let v = v.py_strip();
            if !v.is_empty() && v.len() <= 4300 && v.bytes().all(|b| b.is_ascii_digit()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Python's `_first_nonnegative_int` since b654fad74: a string amount is
    /// ASCII decimal digits within CPython's 4300-digit int limit; anything
    /// else, Unicode digits included, is no amount evidence, never an error.
    #[test]
    fn provider_amounts_take_only_ascii_digits_within_the_int_limit() {
        let read = |value: Py| {
            integer(
                &Py::Dict(vec![(
                    kassiber_pyjson::Key::from("send_amount_msat"),
                    value,
                )]),
                "send_amount_msat",
            )
        };
        assert_eq!(read(Py::from(" 100 ")), Ok(Some(BigInt::from(100))));
        assert_eq!(read(Py::Int(BigInt::from(7))), Ok(Some(BigInt::from(7))));
        assert_eq!(read(Py::Int(BigInt::from(-1))), Ok(None));
        for text in [
            "\u{661}\u{660}\u{660}",
            "\u{b2}",
            "\u{1369}",
            "\u{1d7d9}",
            "1e3",
            "-1",
            "",
            "bad",
        ] {
            assert_eq!(read(Py::from(text)), Ok(None), "{text:?}");
        }
        assert!(read(Py::from("1".repeat(4300).as_str())).unwrap().is_some());
        assert_eq!(read(Py::from("1".repeat(4301).as_str())), Ok(None));
    }
}
