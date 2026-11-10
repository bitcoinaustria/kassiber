use super::*;
use bitcoin_hashes::{hash160, sha256, Hash};
use kassiber_custody::{codec, evidence};
use matcher::{conserves, Row};
use provider::Evidence;

fn bytes(v: &Value) -> Option<Vec<u8>> {
    let Value::Str(v) = v else { return None };
    let v = v.as_str()?;
    // bytes.fromhex accepts ASCII whitespace only between complete bytes.
    let mut result = Vec::new();
    let mut chars = v.bytes().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_whitespace() {
            continue;
        }
        let a = (c as char).to_digit(16)?;
        let b = (chars.next()? as char).to_digit(16)?;
        result.push((a * 16 + b) as u8);
    }
    Some(result)
}
fn template(script: &[u8]) -> Option<&[u8]> {
    let script = script.strip_prefix(&[0x82, 1, 32, 0x88]).unwrap_or(script);
    if script.get(..2) != Some(&[0xa9, 20]) {
        return None;
    }
    let hash = script.get(2..22)?;
    if script.get(22..25) != Some(&[0x87, 0x63, 33]) || script.get(58) != Some(&0x67) {
        return None;
    }
    let head = *script.get(59)?;
    let consumed = match head {
        0 | 0x51..=0x60 => 1,
        1..=5 => 1 + head as usize,
        _ => return None,
    };
    let at = 59 + consumed;
    if script.get(at..at + 3) != Some(&[0xb1, 0x75, 33])
        || script.get(at + 36..) != Some(&[0x68, 0xac])
    {
        return None;
    }
    Some(hash)
}
fn witness(v: &Value) -> Option<(bool, Option<String>)> {
    let Value::List(items) = v else { return None };
    if items.len() < 2 {
        return None;
    }
    let bytes: Vec<_> = items.iter().map(bytes).collect::<Option<_>>()?;
    let hash = template(bytes.last()?)?;
    for b in &bytes[..bytes.len() - 1] {
        if b.len() == 32 && hash160::Hash::hash(b).as_byte_array() == hash {
            return Some((true, Some(sha256::Hash::hash(b).to_string())));
        }
    }
    Some((false, None))
}
fn effective(r: &Row) -> &Value {
    get(&r.raw, "tx")
        .filter(|v| matches!(v, Value::Dict(_)))
        .unwrap_or(&r.raw)
}
fn authoritative(r: &Row, role: &str) -> Option<Value> {
    // Avoid commitment hashing for the common path without an attestation.
    let proof = get(&r.raw, "htlc_spend")?;
    if text(proof, "role") != role
        || !matches!(value(proof,"version"),Value::Int(ref n)if *n==BigInt::from(1))
        || !matches!(value(proof,"input_count"),Value::Int(ref n)if *n==BigInt::from(1))
    {
        return None;
    }
    let observer = st(&r.raw, "observer");
    if !["lwk", "bitcoinrpc"].contains(&observer.as_str()) {
        return None;
    }
    let kinds = parse_json(&st(&r.data, "observation_observer_kinds_json")).ok()?;
    if !list(&kinds).contains(&s(observer)) {
        return None;
    }
    let vins = value(&r.raw, "vin");
    let vins = list(&vins);
    if vins.len() != 1 {
        return None;
    }
    let txid = hex_id(&st(proof, "funding_txid"))?;
    let vout = value(proof, "funding_vout");
    if !matches!(vout, Value::Int(_))
        || n(&vout).ok()? < 0
        || hex_id(&st(&vins[0], "txid")) != Some(txid)
        || value(&vins[0], "vout") != vout
        || !matches!(value(&vins[0], "vout"), Value::Int(_))
    {
        return None;
    }
    evidence::current_observation(&codec::to_json(&r.data))
        .ok()?
        .then(|| proof.clone())
}
fn claim(r: &Row) -> bool {
    if let Some(proof) = authoritative(r, "claim") {
        return hex_id(&st(&proof, "payment_hash")) == r.hash && r.scope.is_some();
    }
    let effective = effective(r);
    let vins = list(get(effective, "vin").unwrap_or(&Value::Null));
    if vins.len() != 1 {
        return false;
    }
    let v = &vins[0];
    if hex_id(&st(v, "txid")).is_none()
        || !matches!(value(v, "vout"), Value::Int(_))
        || n(&value(v, "vout")).unwrap_or(-1) < 0
    {
        return false;
    }
    let w = get(v, "witness").unwrap_or_else(|| get(v, "txinwitness").unwrap_or(&Value::Null));
    witness(w).is_some_and(|(claim, hash)| claim && hash == r.hash) && r.scope.is_some()
}
pub(super) fn hash_role(r: &Row) -> &'static str {
    let source = lower(&st(&r.data, "payment_hash_source"));
    let kind = lower(&st(&r.data, "kind"));
    let provenance = lower(&text(
        &value(&r.raw, "_kassiber_provenance"),
        "import_source",
    ));
    if (source == "lnd"
        && provenance == "lnd"
        && ["lnd_invoice", "lnd_pay"].contains(&kind.as_str()))
        || (source == "core_lightning"
            && provenance == "core-lightning"
            && ["cln_invoice", "cln_pay", "ln_invoice", "ln_pay"].contains(&kind.as_str()))
    {
        return "lightning_node";
    }
    match source.as_str() {
        "chain_script_unique_outpoint" => {
            if claim(r) {
                "chain_htlc"
            } else {
                "chain_htlc_unscoped"
            }
        }
        "chain_script" => "chain_htlc_unscoped",
        "boltz" | "boltz-regtest" | "bullbitcoin" | "generic_ledger" | "importer"
        | "importer_backfill" => "provider_or_import",
        _ => "unknown",
    }
}
fn recovered(r: &Row) -> Option<(String, i128, bool)> {
    if let Some(p) = authoritative(r, "refund") {
        return Some((
            hex_id(&st(&p, "funding_txid"))?,
            n(&value(&p, "funding_vout")).ok()?,
            true,
        ));
    }
    let effective = effective(r);
    let vins = list(get(effective, "vin").unwrap_or(&Value::Null));
    let mut matches = BTreeSet::new();
    for v in vins {
        let w = get(v, "witness")
            .filter(|v| !matches!(v, Value::Null))
            .or_else(|| get(v, "txinwitness"))
            .unwrap_or(&Value::Null);
        if !witness(w).is_some_and(|(claim, _)| !claim) {
            continue;
        }
        let Some(txid) = hex_id(&st(v, "txid")) else {
            continue;
        };
        let Ok(vout) = n(&value(v, "vout")) else {
            continue;
        };
        if vout >= 0 {
            matches.insert((txid, vout));
        }
    }
    if matches.len() != 1 {
        return None;
    }
    let (txid, vout) = matches.into_iter().next()?;
    Some((txid, vout, vins.len() == 1))
}
fn reference(r: &Row) -> (String, Option<i128>, bool) {
    let recovered = recovered(r);
    let proof = |txid: &str, vout: i128| {
        recovered
            .as_ref()
            .is_some_and(|(tx, v, whole)| *whole && hex_id(txid).as_ref() == Some(tx) && vout == *v)
    };
    let outpoint = st(&r.data, "swap_refund_funding_outpoint");
    if !outpoint.is_empty() {
        let (txid, vout) = outpoint
            .split_once(':')
            .map_or((outpoint.as_str(), None), |(t, v)| (t, n(&s(v)).ok()));
        if !txid.is_empty() {
            return (txid.into(), vout, vout.is_some_and(|v| proof(txid, v)));
        }
    }
    let txid = kassiber_pytext::strip(&st(&r.data, "swap_refund_funding_txid")).to_owned();
    if txid.is_empty() {
        return recovered
            .map(|(t, v, p)| (t, Some(v), p))
            .unwrap_or_default();
    }
    if let Some(v) = get(&r.data, "swap_refund_funding_vout")
        .filter(|v| !matches!(v, Value::Null) && **v != s(""))
    {
        if let Ok(v) = n(v) {
            return (txid.clone(), Some(v), proof(&txid, v));
        }
    }
    if let Some((t, v, whole)) = recovered {
        if lower(&t) == lower(&txid) {
            let p = whole && hex_id(&txid) == Some(t.clone());
            return (t, Some(v), p);
        }
    }
    (txid, None, false)
}
fn output_amount(r: &Row, index: i128) -> Option<i128> {
    let mut outputs = value(&r.raw, "vout");
    if !matches!(outputs, Value::List(_)) {
        outputs = value(&value(&r.raw, "tx"), "vout");
    }
    for (position, o) in list(&outputs).iter().enumerate() {
        if !matches!(o, Value::Dict(_)) {
            continue;
        }
        let at = get(o, "n").map(n).unwrap_or(Ok(position as i128)).ok();
        if at != Some(index) {
            continue;
        }
        // The graph's output normalization owns historical BTC/sat spellings.
        let sats = kassiber_custody::components::review::matching_output_sats(&codec::to_json(o))?;
        return (sats >= 0).then_some(i128::from(sats) * 1000);
    }
    None
}
pub(super) fn refund_pairs(
    rows: &[Row],
    outs: &[usize],
    ins: &[usize],
) -> Vec<(usize, usize, Evidence)> {
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for &idx in outs {
        let id = st(&rows[idx].data, "external_id");
        if !id.is_empty() {
            by_id
                .entry(kassiber_pytext::lower(&id))
                .or_default()
                .push(idx);
        }
    }
    let mut result = Vec::new();
    for &ii in ins {
        let i = &rows[ii];
        let (txid, vout, proof) = reference(i);
        if txid.is_empty() {
            continue;
        }
        let funding: Vec<_> = by_id
            .get(&kassiber_pytext::lower(&txid))
            .into_iter()
            .flatten()
            .copied()
            .filter(|&o| compatible(&rows[o].domain, &i.domain))
            .collect();
        if funding.len() != 1 {
            continue;
        }
        let oi = funding[0];
        let o = &rows[oi];
        if kassiber_pytext::upper(&o.asset) != kassiber_pytext::upper(&i.asset) || !conserves(o, i)
        {
            continue;
        }
        let mut version = "txid_legacy";
        if let Some(vout) = vout {
            version = "outpoint_unverified";
            if let Some(amount) = output_amount(o, vout) {
                if amount != o.amount {
                    continue;
                }
                if proof
                    && hex_id(&txid).is_some()
                    && o.scope
                        .as_ref()
                        .is_some_and(|s| Some(s.2.clone()) == hex_id(&txid))
                    && o.scope
                        .as_ref()
                        .zip(i.scope.as_ref())
                        .is_some_and(|(o, i)| o.0 == i.0 && o.1 == i.1)
                {
                    version = "outpoint_exact";
                }
            }
        }
        result.push((
            oi,
            ii,
            Evidence {
                provider: "chain_htlc".into(),
                id: vout.map(|v| format!("{txid}:{v}")).unwrap_or(txid),
                kind: "swap-refund".into(),
                flow: "refund".into(),
                status: "refunded".into(),
                version: version.into(),
                spend_path: "timeout".into(),
                ..Evidence::default()
            },
        ));
    }
    result
}
