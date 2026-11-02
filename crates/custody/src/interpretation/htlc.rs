//! Non-secret observer attestations and legacy public witness replay.
use super::recorded;
use crate::{
    codec, evidence,
    graph::{self, stored as s},
    python_text::PythonText,
};
use bitcoin_hashes::{hash160, sha256, Hash};
use kassiber_pyjson::{BigInt, Value as Py};
use serde_json::Value;

pub(super) fn raw(row: &Value) -> Py {
    s::object(&row["raw_json"])
}
fn tx(payload: &Py) -> &Py {
    let nested = s::get(payload, "tx");
    if matches!(nested, Py::Dict(_)) {
        nested
    } else {
        payload
    }
}
fn txid(value: &Py) -> Option<String> {
    graph::strict_txid(&codec::to_json(value))
}
fn uint(value: &Py) -> Option<BigInt> {
    match value {
        Py::Int(n) if *n >= BigInt::from(0) => Some(n.clone()),
        _ => None,
    }
}
fn attestation(row: &Value, role: &str) -> Result<Option<Py>, String> {
    if !evidence::current_observation(row).map_err(|e| e.code.to_owned())? {
        return Ok(None);
    }
    let kinds = row["observation_observer_kinds_json"]
        .as_str()
        .and_then(|v| codec::loads(v).ok());
    let Some(Py::List(kinds)) = kinds else {
        return Ok(None);
    };
    let payload = raw(row);
    let observer = s::get(&payload, "observer");
    if matches!(observer, Py::List(_) | Py::Dict(_)) {
        return Err("TypeError".into());
    }
    if !matches!(observer, Py::Str(v) if matches!(v.as_str(),Some("lwk"|"bitcoinrpc")))
        || !kinds.contains(observer)
    {
        return Ok(None);
    }
    let proof = s::get(&payload, "htlc_spend");
    if !matches!(proof, Py::Dict(_)) || s::get(proof, "role") != &Py::Str(role.into()) {
        return Ok(None);
    }
    if !matches!(s::get(proof,"version"), Py::Int(n) if *n == BigInt::from(1))
        || !matches!(s::get(proof,"input_count"), Py::Int(n) if *n == BigInt::from(1))
    {
        return Ok(None);
    }
    let Py::List(vins) = s::get(&payload, "vin") else {
        return Ok(None);
    };
    if vins.len() != 1 || !matches!(&vins[0], Py::Dict(_)) {
        return Ok(None);
    }
    let Some(funding) = txid(s::get(proof, "funding_txid")) else {
        return Ok(None);
    };
    let Some(vout) = uint(s::get(proof, "funding_vout")) else {
        return Ok(None);
    };
    if txid(s::get(&vins[0], "txid")) != Some(funding)
        || uint(s::get(&vins[0], "vout")) != Some(vout)
    {
        return Ok(None);
    }
    Ok(Some(proof.clone()))
}
// bytes.fromhex accepts ASCII whitespace only between complete bytes.
fn unhex(value: &Py) -> Option<Vec<u8>> {
    let Py::Str(v) = value else {
        return None;
    };
    let text = v.as_str()?;
    let mut bytes = text.bytes().peekable();
    let mut result = Vec::new();
    while let Some(a) = bytes.next() {
        if matches!(a, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c') {
            continue;
        }
        let a = (a as char).to_digit(16)?;
        let b = (bytes.next()? as char).to_digit(16)?;
        result.push((a * 16 + b) as u8);
    }
    Some(result)
}
fn script(bytes: &[u8]) -> Result<Option<&[u8]>, String> {
    let at = if bytes.starts_with(&[0x82, 1, 32, 0x88]) {
        4
    } else {
        0
    };
    // Preserve the reference parser's indexed OP_EQUAL read at this boundary.
    if bytes.len() == at + 22 && bytes.get(at..at + 2) == Some(&[0xa9, 20]) {
        return Err("IndexError".into());
    }
    Ok(script_shape(bytes))
}
fn script_shape(bytes: &[u8]) -> Option<&[u8]> {
    let mut at = if bytes.starts_with(&[0x82, 1, 32, 0x88]) {
        4
    } else {
        0
    };
    if bytes.get(at..at + 2)? != [0xa9, 20] {
        return None;
    }
    at += 2;
    let hash = bytes.get(at..at + 20)?;
    at += 20;
    if bytes.get(at..at + 3)? != [0x87, 0x63, 33] {
        return None;
    }
    at += 3 + 33;
    if *bytes.get(at)? != 0x67 {
        return None;
    }
    at += 1;
    let head = *bytes.get(at)?;
    at += 1;
    match head {
        0 | 0x51..=0x60 => {}
        1..=5 => {
            bytes.get(at..at + head as usize)?;
            at += head as usize;
        }
        _ => return None,
    }
    if bytes.get(at..at + 3)? != [0xb1, 0x75, 33] {
        return None;
    }
    at += 3 + 33;
    if bytes.get(at..)? != [0x68, 0xac] {
        return None;
    }
    Some(hash)
}
fn witness(value: &Py) -> Result<Option<(bool, Option<String>)>, String> {
    let Py::List(items) = value else {
        return Ok(None);
    };
    let Some(items) = items.iter().map(unhex).collect::<Option<Vec<_>>>() else {
        return Ok(None);
    };
    if items.len() < 2 {
        return Ok(None);
    }
    let Some(expected) = script(items.last().unwrap())? else {
        return Ok(None);
    };
    for item in &items[..items.len() - 1] {
        if item.len() == 32 && hash160::Hash::hash(item).as_byte_array() == expected {
            return Ok(Some((true, Some(sha256::Hash::hash(item).to_string()))));
        }
    }
    Ok(Some((false, None)))
}
fn witness_field(vin: &Py, null_fallback: bool) -> &Py {
    let w = s::get(vin, "witness");
    let present = matches!(vin,Py::Dict(entries) if entries.iter().any(|(k,_)| matches!(k,kassiber_pyjson::Key::Str(k) if k.as_str()==Some("witness"))));
    if !present || null_fallback && matches!(w, Py::Null) {
        s::get(vin, "txinwitness")
    } else {
        w
    }
}
pub(super) fn claim(row: &Value) -> Result<bool, String> {
    if let Some(proof) = attestation(row, "claim")? {
        return Ok(codec::to_json(s::get(&proof, "payment_hash"))
            == serde_json::json!(
                recorded::canonical_hash(&row["payment_hash"]).unwrap_or_default()
            )
            && graph::onchain_scope(row).is_some());
    }
    let payload = raw(row);
    let Py::List(vins) = s::get(tx(&payload), "vin") else {
        return Ok(false);
    };
    if vins.len() != 1
        || !matches!(&vins[0], Py::Dict(_))
        || txid(s::get(&vins[0], "txid")).is_none()
        || uint(s::get(&vins[0], "vout")).is_none()
    {
        return Ok(false);
    }
    Ok(
        witness(witness_field(&vins[0], false))?.is_some_and(|(is_claim, hash)| {
            is_claim && hash == recorded::canonical_hash(&row["payment_hash"])
        }) && graph::onchain_scope(row).is_some(),
    )
}
fn recovered(row: &Value) -> Result<(Option<(String, BigInt)>, bool), String> {
    if let Some(proof) = attestation(row, "refund")? {
        return Ok((
            Some((
                s::text(s::get(&proof, "funding_txid")),
                uint(s::get(&proof, "funding_vout")).unwrap(),
            )),
            true,
        ));
    }
    let payload = raw(row);
    let vins = s::get(tx(&payload), "vin");
    let mut matches = Vec::new();
    for vin in s::list(vins) {
        if !matches!(vin, Py::Dict(_)) || !witness(witness_field(vin, true))?.is_some_and(|v| !v.0)
        {
            continue;
        }
        let Some(txid) = txid(s::get(vin, "txid")) else {
            continue;
        };
        let vout = match s::integer(s::get(vin, "vout")) {
            Ok(n) => n,
            Err("TypeError" | "ValueError") => continue,
            Err(e) => return Err(e.into()),
        };
        if vout < BigInt::from(0) {
            continue;
        }
        let key = (txid, vout);
        if !matches.contains(&key) {
            matches.push(key);
        }
    }
    Ok((
        if matches.len() == 1 {
            matches.pop()
        } else {
            None
        },
        matches!(vins,Py::List(v) if v.len()==1),
    ))
}
pub(super) fn refund(row: &Value) -> Result<(String, Option<BigInt>, bool), String> {
    let (recovered, whole) = recovered(row)?;
    let proves = |id: &str, vout: &BigInt| {
        whole
            && recovered.as_ref().is_some_and(|(t, n)| {
                graph::strict_txid(&serde_json::json!(id)).as_ref() == Some(t) && vout == n
            })
    };
    if codec::truth(&row["swap_refund_funding_outpoint"]) {
        let outpoint = codec::pystr(&row["swap_refund_funding_outpoint"]);
        let (id, index) = outpoint
            .split_once(':')
            .map_or((outpoint.as_str(), None), |(a, b)| (a, Some(b)));
        if !id.is_empty() {
            let vout = index
                .map(|v| s::integer(&Py::Str(v.into())))
                .transpose()
                .ok()
                .flatten();
            let proven = vout.as_ref().is_some_and(|n| proves(id, n));
            return Ok((id.into(), vout, proven));
        }
    }
    let id = codec::text(&row["swap_refund_funding_txid"])
        .py_strip()
        .to_owned();
    if id.is_empty() {
        return Ok(recovered.map_or((String::new(), None, false), |(t, n)| (t, Some(n), whole)));
    }
    let raw_index = &row["swap_refund_funding_vout"];
    if !raw_index.is_null() && raw_index != "" {
        match s::integer(&codec::from_json(raw_index)?) {
            Ok(vout) => {
                let proof = proves(&id, &vout);
                return Ok((id, Some(vout), proof));
            }
            Err("TypeError" | "ValueError") => {}
            Err(e) => return Err(e.into()),
        }
    }
    if let Some((t, n)) = recovered {
        if t == id.py_strip().py_lower() {
            let proof = whole && graph::strict_txid(&serde_json::json!(id)) == Some(t.clone());
            return Ok((t, Some(n), proof));
        }
    }
    Ok((id, None, false))
}
pub(super) fn output(row: &Value, vout: &BigInt) -> Result<Option<BigInt>, String> {
    let payload = raw(row);
    let outputs = match s::get(&payload, "vout") {
        v @ Py::List(_) => v,
        _ => s::get(tx(&payload), "vout"),
    };
    for (position, out) in s::list(outputs).iter().enumerate() {
        if !matches!(out, Py::Dict(_)) {
            continue;
        }
        let index = if matches!(out,Py::Dict(v) if !v.iter().any(|(k,_)| matches!(k,kassiber_pyjson::Key::Str(k) if k.as_str()==Some("n"))))
        {
            BigInt::from(position)
        } else {
            match s::integer(s::get(out, "n")) {
                Ok(n) => n,
                Err("TypeError" | "ValueError") => continue,
                Err(e) => return Err(e.into()),
            }
        };
        if &index != vout {
            continue;
        }
        // Reuse the stored graph's Python-compatible sats/decimal parser.
        return Ok(crate::graph::stored::output_sats(out)
            .filter(|n| *n >= BigInt::from(0))
            .map(|n| n * BigInt::from(1000)));
    }
    Ok(None)
}
pub(super) fn presence<R: crate::fields::Fields>(rows: &[R]) -> bool {
    rows.iter().any(|row| {
        if codec::truth(row.at("swap_refund_funding_txid"))
            || codec::truth(row.at("swap_refund_funding_outpoint"))
            || *row.at("payment_hash_source") == "chain_script_unique_outpoint"
        {
            return true;
        }
        let raw = row.at("raw_json");
        if raw
            .as_str()
            .is_some_and(|v| !v.contains("htlc_spend") && !v.contains("witness"))
        {
            return false;
        }
        let payload = s::object(raw);
        s::truth(s::get(&payload, "htlc_spend"))
            || s::list(s::get(tx(&payload), "vin"))
                .iter()
                .any(|vin| s::truth(s::get(vin, "witness")) || s::truth(s::get(vin, "txinwitness")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_htlc_helper_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_native_helpers.json"
        ))
        .unwrap();
        for vector in vectors
            .iter()
            .filter(|v| v["op"] != "matcher" && v["op"] != "production" && v["op"] != "finalize")
        {
            let v = &vector["value"];
            let result = (|| -> Result<Value, String> {
                Ok(match vector["op"].as_str().unwrap() {
                    "claim" => serde_json::json!(claim(v)?),
                    "refund" => {
                        let (t, n, p) = refund(v)?;
                        serde_json::json!([t, n.as_ref().map(super::super::matcher::number), p])
                    }
                    "presence" => serde_json::json!(presence(super::super::array(v))),
                    "output" => serde_json::json!(output(
                        &v["row"],
                        &s::integer(&codec::from_json(&v["index"])?).map_err(str::to_owned)?
                    )?
                    .as_ref()
                    .map(super::super::matcher::number)),
                    "script" => {
                        let bytes = unhex(&codec::from_json(v)?).unwrap();
                        serde_json::json!(script(&bytes)?.map(|bytes| bytes
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()))
                    }
                    "witness" => serde_json::json!(witness(&codec::from_json(v)?)?),
                    _ => unreachable!(),
                })
            })();
            if vector.get("error").is_some() {
                assert_eq!(
                    result.err().map(|e| serde_json::json!(e)),
                    Some(vector["error"].clone()),
                    "{}",
                    vector["case"]
                );
            } else {
                assert_eq!(result.unwrap(), vector["expected"], "{}", vector["case"]);
            }
        }
    }
}
