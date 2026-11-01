//! Local proposal comparison and explicit BIP78 constraint checks.
//! Signature checks verify supported commitments only; nothing is signed.
use super::*;
use bitcoin::hashes::sha256d;
use bitcoin::secp256k1::{ecdsa, Message, PublicKey, Secp256k1};
use kassiber_pyjson::{BigInt, Key, Value as Py};
use std::collections::{BTreeMap, BTreeSet};

/// A PSBT argument as supplied: local file bytes or a JSON request value.
pub enum Payload<'a> {
    Bytes(&'a [u8]),
    Json(&'a Py),
}

pub fn decode(payload: &Payload<'_>) -> Result<Vec<u8>> {
    match payload {
        Payload::Bytes(raw) => super::payload(raw),
        Payload::Json(Py::Str(text)) => match text.as_str() {
            Some(text) => text_bytes(text),
            None if text.code_points().count() > MAX_PSBT_BYTES * 3 => {
                Err(invalid("payload_limit"))
            }
            None => Err(invalid("invalid_base64_or_hex")),
        },
        Payload::Json(_) => Err(invalid("invalid_payload_type")),
    }
}

fn network(value: &Py) -> Result<&str> {
    match value {
        Py::Str(v)
            if v.as_str()
                .is_some_and(|v| ["main", "test", "signet", "regtest"].contains(&v)) =>
        {
            Ok(v.as_str().unwrap())
        }
        _ => Err(AppError::new(
            "chain_analysis_invalid_network",
            "Select an explicit Bitcoin network.",
        )),
    }
}

/// `analyze_psbt`: decoding precedes the network check.
pub fn analyze_payload(payload: &Payload<'_>, network_value: &Py, subject: &str) -> Result<Py> {
    let data = decode(payload)?;
    parse(&data)?;
    let network = network(network_value)?;
    Ok(order::analysis(&analysis(&data, network, subject)?))
}

/// The ordered public analysis, plus its serde form for entropy facts.
pub fn analyze_parts(
    payload: &Payload<'_>,
    network_value: &Py,
    subject: &str,
) -> Result<(Py, Value)> {
    let data = decode(payload)?;
    parse(&data)?;
    let network = network(network_value)?;
    let raw = analysis(&data, network, subject)?;
    Ok((order::analysis(&raw), raw))
}

fn invalid_constraints(message: &str) -> AppError {
    AppError::new("chain_analysis_invalid_payjoin_constraints", message)
}
fn num(n: impl Into<BigInt>) -> Py {
    Py::Int(n.into())
}
fn string(v: impl ToString) -> Py {
    s(v.to_string())
}
fn index(value: Option<&Py>, len: usize) -> Option<usize> {
    match value {
        Some(Py::Int(n)) => usize::try_from(n.clone()).ok().filter(|n| *n < len),
        _ => None,
    }
}

pub fn compare(
    before: &Payload<'_>,
    after: &Payload<'_>,
    network_value: &Py,
    payjoin: Option<&Py>,
) -> Result<Py> {
    let network = network(network_value)?;
    let (before, after) = (decode(before)?, decode(after)?);
    // The reference decodes both proposals before analyzing either.
    let (a, b) = (parse(&before)?, parse(&after)?);
    let left = analysis(&before, network, "psbt:before")?;
    let right = analysis(&after, network, "psbt:after")?;
    let features = |v: &Value| {
        let mut out: Vec<(String, Value)> = Vec::new();
        for row in v["features"]["features"].as_array().into_iter().flatten() {
            let code = row["code"].as_str().unwrap_or("").to_owned();
            match out.iter_mut().find(|(k, _)| *k == code) {
                Some(slot) => slot.1 = row["value"].clone(),
                None => out.push((code, row["value"].clone())),
            }
        }
        out
    };
    let (lf, rf) = (features(&left), features(&right));
    let ids = |v: &Value| -> BTreeSet<String> {
        v["transaction_facts"]["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| r["output_id"].as_str().unwrap_or("").to_owned())
            .collect()
    };
    let (li, ri) = (ids(&left), ids(&right));
    let scripts = |p: &Parsed<'_>| {
        let mut out: BTreeMap<Vec<u8>, i64> = BTreeMap::new();
        for o in &p.tx.outputs {
            *out.entry(o.script.to_vec()).or_default() += 1;
        }
        out
    };
    let (lo, ro) = (scripts(&a), scripts(&b));
    let surplus = |x: &BTreeMap<Vec<u8>, i64>, y: &BTreeMap<Vec<u8>, i64>| -> i64 {
        x.iter()
            .map(|(k, n)| (n - y.get(k).copied().unwrap_or(0)).max(0))
            .sum()
    };
    let codes = |v: &Value| -> BTreeSet<String> {
        v["findings"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| r["code"].as_str().unwrap_or("").to_owned())
            .collect()
    };
    let (lc, rc) = (codes(&left), codes(&right));
    let fee = |v: &Value| {
        v["totals"]["fee_msat"]
            .as_str()
            .map(|f| f.parse::<i128>().unwrap())
    };
    let delta = fee(&left).zip(fee(&right)).map(|(a, b)| b - a);
    let strings = |set: Vec<&String>| Py::List(set.into_iter().map(|v| s(v.as_str())).collect());
    let (la, ra) = (
        order::adapter_signatures(&left),
        order::adapter_signatures(&right),
    );
    let changed = lf
        .iter()
        .filter(|(k, v)| {
            rf.iter()
                .find(|(key, _)| key == k)
                .map_or(&Value::Null, |(_, v)| v)
                != v
        })
        .map(|(k, v)| {
            let after = rf
                .iter()
                .find(|(key, _)| key == k)
                .map_or(Py::Null, |(_, v)| {
                    order::feature_value(k, &order::py(v), ra)
                });
            dict([
                ("code", s(k.as_str())),
                ("before", order::feature_value(k, &order::py(v), la)),
                ("after", after),
            ])
        })
        .collect();
    let mut result = dict([
        ("schema_version", num(1)),
        ("before", order::analysis(&left)),
        ("after", order::analysis(&right)),
        (
            "delta",
            dict([
                ("inputs_added", strings(ri.difference(&li).collect())),
                ("inputs_removed", strings(li.difference(&ri).collect())),
                ("output_scripts_added", num(surplus(&ro, &lo))),
                ("output_scripts_removed", num(surplus(&lo, &ro))),
                ("fee_msat", delta.map_or(Py::Null, string)),
                ("features_changed", Py::List(changed)),
                ("findings_added", strings(rc.difference(&lc).collect())),
                ("findings_removed", strings(lc.difference(&rc).collect())),
            ]),
        ),
    ]);
    if let Some(constraints) = payjoin.filter(|v| **v != Py::Null) {
        let checks = payjoin_checks(&a, &b, &left, &right, constraints, delta)?;
        kassiber_envelope::set(&mut result, "payjoin", checks);
    }
    Ok(result)
}

fn sighash(tx: &Transaction<'_>, index: usize, key_hash: &[u8], value: u64, flag: u8) -> [u8; 32] {
    let (base, anyone) = (flag & 0x1f, flag & 0x80 != 0);
    let double = |data: &[u8]| sha256d::Hash::hash(data).to_byte_array();
    let mut data = Vec::new();
    data.extend_from_slice(&(tx.version as u32).to_le_bytes());
    let mut prevouts = Vec::new();
    let mut sequences = Vec::new();
    for i in &tx.inputs {
        prevouts.extend_from_slice(i.txid);
        prevouts.extend_from_slice(&(i.vout as u32).to_le_bytes());
        sequences.extend_from_slice(&(i.sequence as u32).to_le_bytes());
    }
    data.extend_from_slice(&if anyone { [0; 32] } else { double(&prevouts) });
    data.extend_from_slice(&if anyone || base == 2 || base == 3 {
        [0; 32]
    } else {
        double(&sequences)
    });
    let input = &tx.inputs[index];
    data.extend_from_slice(input.txid);
    data.extend_from_slice(&(input.vout as u32).to_le_bytes());
    let mut code = vec![0x76, 0xa9, 0x14];
    code.extend_from_slice(key_hash);
    code.extend_from_slice(&[0x88, 0xac]);
    variable(&code, &mut data);
    data.extend_from_slice(&value.to_le_bytes());
    data.extend_from_slice(&(input.sequence as u32).to_le_bytes());
    let output = |o: &Output<'_>, out: &mut Vec<u8>| {
        out.extend_from_slice(&o.amount.to_le_bytes());
        variable(o.script, out);
    };
    if base != 2 && base != 3 {
        let mut outputs = Vec::new();
        for o in &tx.outputs {
            output(o, &mut outputs);
        }
        data.extend_from_slice(&double(&outputs));
    } else if base == 3 && index < tx.outputs.len() {
        let mut single = Vec::new();
        output(&tx.outputs[index], &mut single);
        data.extend_from_slice(&double(&single));
    } else {
        data.extend_from_slice(&[0; 32]);
    }
    data.extend_from_slice(&(tx.locktime as u32).to_le_bytes());
    data.extend_from_slice(&u32::from(flag).to_le_bytes());
    double(&data)
}

fn verify_keyhash(p: &Parsed<'_>, index: usize) -> Result<Option<bool>> {
    let (row, input) = (&p.inputs[index], &p.tx.inputs[index]);
    let Some(utxo) = input_utxo(row, input, index)?.0 else {
        return Ok(None);
    };
    let mut outer = utxo.script;
    if kind(outer) == "p2sh" {
        let Some(pushes) = field(row, 7).and_then(crate::analysis_features::pushes) else {
            return Ok(None);
        };
        if pushes.len() != 1 {
            return Ok(None);
        }
        if hash160::Hash::hash(pushes[0]).as_byte_array() != &outer[2..22] {
            return Ok(Some(false));
        }
        outer = pushes[0];
    } else if field(row, 7).is_some_and(|s| !s.is_empty()) {
        return Ok(if kind(outer) == "p2wpkh" {
            Some(false)
        } else {
            None
        });
    }
    if kind(outer) != "p2wpkh" {
        return Ok(None);
    }
    let Some(raw) = field(row, 8) else {
        return Ok(Some(false));
    };
    let stack = witness(raw)?;
    if stack.len() != 2 || hash160::Hash::hash(stack[1]).as_byte_array() != &outer[2..] {
        return Ok(Some(false));
    }
    let Some(encoding) = signature_encoding(stack[0], false) else {
        return Ok(Some(false));
    };
    let flag = encoding["sighash"].as_u64().unwrap_or(0);
    if ![1, 2, 3, 0x81, 0x82, 0x83].contains(&flag) {
        return Ok(Some(false));
    }
    let key = stack[1];
    if !(key.len() == 33 && [2, 3].contains(&key[0]) || key.len() == 65 && key[0] == 4) {
        return Ok(Some(false));
    }
    let (Ok(key), Ok(signature)) = (
        PublicKey::from_slice(key),
        ecdsa::Signature::from_der(&stack[0][..stack[0].len() - 1]),
    ) else {
        return Ok(Some(false));
    };
    let digest = sighash(&p.tx, index, &outer[2..], utxo.amount, flag as u8);
    Ok(Some(
        Secp256k1::verification_only()
            .verify_ecdsa(&Message::from_digest(digest), &signature, &key)
            .is_ok(),
    ))
}

fn final_input_vsize(p: &Parsed<'_>, index: usize) -> Result<Option<usize>> {
    let (row, input) = (&p.inputs[index], &p.tx.inputs[index]);
    let utxo = input_utxo(row, input, index)?.0;
    let (scriptsig, stack) = (field(row, 7), field(row, 8));
    let Some(utxo) = utxo.filter(|_| scriptsig.is_some() || stack.is_some()) else {
        return Ok(None);
    };
    let kind = kind(utxo.script);
    if !["p2pkh", "p2wpkh", "p2sh", "p2tr"].contains(&kind) {
        return Ok(None);
    }
    let scriptsig = scriptsig.unwrap_or(&[]);
    if kind == "p2sh" {
        let pushes = crate::analysis_features::pushes(scriptsig).unwrap_or_default();
        if pushes.len() != 1 || super::kind(pushes[0]) != "p2wpkh" {
            return Ok(None);
        }
    }
    if kind != "p2pkh" && stack.is_none() {
        return Ok(None);
    }
    let base = 40 + compact_width(scriptsig.len()) + scriptsig.len();
    Ok(Some((base * 4 + stack.map_or(0, <[u8]>::len)).div_ceil(4)))
}

fn decimal(text: &str) -> Decimal {
    text.parse().expect("canonical fee-rate decimal")
}

fn payjoin_checks(
    before: &Parsed<'_>,
    after: &Parsed<'_>,
    left: &Value,
    right: &Value,
    constraints: &Py,
    delta: Option<i128>,
) -> Result<Py> {
    let allowed = [
        "payment_output_index",
        "additional_fee_output_index",
        "max_additional_fee_contribution_msat",
        "allow_output_substitution",
        "minimum_fee_rate_sat_vb",
    ];
    let Py::Dict(items) = constraints else {
        return Err(invalid_constraints(
            "Invalid Payjoin comparison constraints.",
        ));
    };
    if items
        .iter()
        .any(|(k, _)| !matches!(k, Key::Str(k) if k.as_str().is_some_and(|k| allowed.contains(&k))))
    {
        return Err(invalid_constraints(
            "Invalid Payjoin comparison constraints.",
        ));
    }
    let get = |k: &str| kassiber_envelope::get(constraints, k);
    let outputs = before.tx.outputs.len();
    let payment = index(get("payment_output_index"), outputs).ok_or_else(|| {
        invalid_constraints("Select the original payment output for Payjoin comparison.")
    })?;
    let fee_output = match get("additional_fee_output_index") {
        None | Some(Py::Null) => None,
        value => Some(
            index(value, outputs)
                .filter(|i| *i != payment)
                .ok_or_else(|| invalid_constraints("Invalid additional fee output."))?,
        ),
    };
    let maximum_text = match get("max_additional_fee_contribution_msat") {
        None => "0".to_owned(),
        Some(Py::Str(v)) => v.as_str().unwrap_or("x").to_owned(),
        Some(_) => "x".to_owned(),
    };
    let maximum = Some(&maximum_text)
        .filter(|v| !v.is_empty() && v.len() <= 19 && v.bytes().all(|b| b.is_ascii_digit()))
        .map(|v| v.parse::<u128>().unwrap())
        .filter(|v| *v <= u128::from(MAX_MONEY) * 1000 && v % 1000 == 0)
        .ok_or_else(|| {
            invalid_constraints(
                "Fee contribution must be an exact nonnegative whole-satoshi msat string.",
            )
        })?;
    let substitute = match get("allow_output_substitution") {
        None => false,
        Some(Py::Bool(v)) => *v,
        Some(_) => {
            return Err(invalid_constraints(
                "Output substitution must be a boolean.",
            ))
        }
    };
    let mut checks: Vec<Py> = Vec::new();
    let mut check = |code: &str, passed: Option<bool>, details: Vec<(&str, Py)>| {
        let status = match passed {
            None => "unavailable",
            Some(true) => "passed",
            Some(false) => "failed",
        };
        checks.push(dict([
            ("code", s(code)),
            ("status", s(status)),
            ("details", dict(details)),
        ]));
    };
    let point = |i: &Input<'_>| (i.txid.to_vec(), i.vout);
    let a_points: Vec<_> = before.tx.inputs.iter().map(point).collect();
    let b_points: Vec<_> = after.tx.inputs.iter().map(point).collect();
    let in_b = |p: &(Vec<u8>, u64)| b_points.iter().position(|q| q == p);
    let in_a = |p: &(Vec<u8>, u64)| a_points.contains(p);
    check(
        "original_inputs_preserved",
        Some(a_points.iter().all(|p| in_b(p).is_some())),
        vec![],
    );
    check(
        "original_input_order_preserved",
        Some(b_points.iter().filter(|p| in_a(p)).eq(a_points.iter())),
        vec![],
    );
    check(
        "transaction_version_unchanged",
        Some(before.tx.version == after.tx.version),
        vec![],
    );
    check(
        "locktime_unchanged",
        Some(before.tx.locktime == after.tx.locktime),
        vec![],
    );
    check(
        "sender_sequences_unchanged",
        Some(
            before.tx.inputs.iter().all(|i| {
                in_b(&point(i)).is_some_and(|j| after.tx.inputs[j].sequence == i.sequence)
            }),
        ),
        vec![],
    );
    check(
        "proposal_sequences_uniform",
        Some(
            after
                .tx
                .inputs
                .iter()
                .map(|i| i.sequence)
                .collect::<BTreeSet<_>>()
                .len()
                <= 1,
        ),
        vec![],
    );
    let metadata = right["validation"]["input_metadata"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let truth = |v: &Value| v.as_bool().unwrap_or(false) || v.as_u64().is_some_and(|n| n != 0);
    check(
        "no_input_derivations",
        Some(
            !metadata
                .iter()
                .any(|r| truth(&r["derivation_metadata_present"])),
        ),
        vec![],
    );
    check(
        "no_output_derivations",
        Some(!truth(
            &right["validation"]["output_derivation_metadata_present"],
        )),
        vec![],
    );
    check(
        "no_partial_signatures",
        Some(
            !metadata
                .iter()
                .any(|r| truth(&r["partial_signature_count"])),
        ),
        vec![],
    );
    check(
        "sender_inputs_unfinalized",
        Some(
            !b_points
                .iter()
                .enumerate()
                .any(|(i, p)| in_a(p) && truth(&metadata[i]["finalized"])),
        ),
        vec![],
    );
    let receivers: Vec<usize> = b_points
        .iter()
        .enumerate()
        .filter(|(_, p)| !in_a(p))
        .map(|(i, _)| i)
        .collect();
    let facts = right["transaction_facts"]["inputs"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    check(
        "receiver_inputs_finalized",
        Some(receivers.iter().all(|i| truth(&metadata[*i]["finalized"]))),
        vec![],
    );
    let supplied = receivers
        .iter()
        .all(|i| !facts[*i]["amount_msat"].is_null());
    check("receiver_utxos_supplied", Some(supplied), vec![]);
    let signatures = receivers
        .iter()
        .map(|i| verify_keyhash(after, *i))
        .collect::<Result<Vec<_>>>()?;
    check(
        "receiver_signature_commitments_valid",
        if signatures.contains(&Some(false)) {
            Some(false)
        } else if signatures.contains(&None) {
            None
        } else {
            Some(true)
        },
        vec![(
            "supported_templates",
            Py::List(vec![s("p2wpkh"), s("p2sh-p2wpkh")]),
        )],
    );
    let mut consistent = Some(true);
    for (i, p) in a_points.iter().enumerate() {
        let Some(j) = in_b(p) else { continue };
        let x = input_utxo(&before.inputs[i], &before.tx.inputs[i], i)?.0;
        let y = input_utxo(&after.inputs[j], &after.tx.inputs[j], j)?.0;
        match (x, y) {
            (Some(x), Some(y)) => {
                if x.amount != y.amount || x.script != y.script {
                    consistent = Some(false);
                }
            }
            _ => {
                if consistent == Some(true) {
                    consistent = None;
                }
            }
        }
    }
    check("sender_prevouts_unchanged", consistent, vec![]);
    check("absolute_fee_not_decreased", delta.map(|d| d >= 0), vec![]);
    let protected: Vec<(usize, &Output<'_>)> = before
        .tx
        .outputs
        .iter()
        .enumerate()
        .filter(|(i, _)| !(*i == payment && substitute))
        .collect();
    let mut original: Vec<(&[u8], usize)> = Vec::new();
    for (_, o) in &protected {
        match original.iter_mut().find(|(k, _)| *k == o.script) {
            Some(slot) => slot.1 += 1,
            None => original.push((o.script, 1)),
        }
    }
    let proposal = |script: &[u8]| {
        after
            .tx
            .outputs
            .iter()
            .filter(|o| o.script == script)
            .count()
    };
    let counts_preserved = original.iter().all(|(k, n)| proposal(k) >= *n);
    let mut required: Vec<(&[u8], i128)> = Vec::new();
    for (i, o) in &protected {
        let amount = i128::from(o.amount) * 1000;
        let reduce = if Some(*i) == fee_output {
            amount.min(maximum as i128)
        } else {
            0
        };
        match required.iter_mut().find(|(k, _)| *k == o.script) {
            Some(slot) => slot.1 += amount - reduce,
            None => required.push((o.script, amount - reduce)),
        }
    }
    let proposed = |script: &[u8]| -> i128 {
        after
            .tx
            .outputs
            .iter()
            .filter(|o| o.script == script)
            .map(|o| i128::from(o.amount) * 1000)
            .sum()
    };
    let values_preserved = required.iter().all(|(k, n)| proposed(k) >= *n);
    check(
        "protected_output_cardinality_preserved",
        Some(counts_preserved),
        vec![],
    );
    check(
        "protected_output_aggregate_value_preserved",
        Some(values_preserved),
        vec![],
    );
    let unique = original.iter().all(|(k, n)| *n == 1 && proposal(k) <= 1);
    check(
        "output_correspondence_unambiguous",
        unique.then_some(true),
        vec![],
    );
    if unique {
        let position = |script: &[u8]| after.tx.outputs.iter().rposition(|o| o.script == script);
        let positions: Vec<usize> = protected
            .iter()
            .filter_map(|(_, o)| position(o.script))
            .collect();
        check(
            "original_output_order_preserved",
            Some(counts_preserved && positions.windows(2).all(|w| w[0] <= w[1])),
            vec![("payment_output_substitution_allowed", Py::Bool(substitute))],
        );
        let mut contribution: i128 = 0;
        let mut outputs_ok = true;
        for (i, o) in &protected {
            match position(o.script).map(|p| &after.tx.outputs[p]) {
                None => outputs_ok = false,
                Some(m) if Some(*i) == fee_output => {
                    contribution = (i128::from(o.amount) - i128::from(m.amount)).max(0) * 1000;
                    outputs_ok &= contribution <= maximum as i128;
                }
                Some(m) => outputs_ok &= m.amount >= o.amount,
            }
        }
        check(
            "sender_outputs_preserved_and_values_valid",
            Some(outputs_ok),
            vec![],
        );
        check(
            "fee_contribution_within_budget",
            Some(contribution <= maximum as i128),
            vec![
                ("contribution_msat", string(contribution)),
                ("maximum_msat", s(maximum_text.as_str())),
            ],
        );
        check(
            "fee_contribution_only_pays_fee_increase",
            delta.map(|d| contribution <= d),
            vec![],
        );
        let sizes = (0..before.tx.inputs.len())
            .map(|i| final_input_vsize(before, i))
            .collect::<Result<Vec<_>>>()?;
        let rate = left["totals"]["final_fee_rate_sat_vb"].as_str();
        let cap = match (rate, sizes.first()) {
            (Some(rate), Some(Some(first))) if sizes.iter().all(|v| *v == Some(*first)) => {
                let context = Context {
                    prec: 32,
                    rounding: Rounding::HalfEven,
                };
                let mut cap = decimal(rate);
                for factor in [*first, receivers.len(), 1000] {
                    cap = cap.mul(&Decimal::from(factor as u64), &context).unwrap();
                }
                Some(cap)
            }
            _ => None,
        };
        let passed = match &cap {
            Some(cap) => Some(decimal(&contribution.to_string()) <= *cap),
            None if contribution == 0 => Some(true),
            None => None,
        };
        check(
            "fee_contribution_added_input_cost_cap",
            passed,
            vec![("maximum_msat", cap.map_or(Py::Null, |c| s(c.to_string())))],
        );
    } else {
        check(
            "original_output_order_preserved",
            None,
            vec![("payment_output_substitution_allowed", Py::Bool(substitute))],
        );
        check(
            "sender_outputs_preserved_and_values_valid",
            if counts_preserved && values_preserved {
                None
            } else {
                Some(false)
            },
            vec![],
        );
    }
    if let Some(minimum) = get("minimum_fee_rate_sat_vb").filter(|v| **v != Py::Null) {
        let invalid_rate =
            || invalid_constraints("Minimum fee rate must be a nonnegative decimal string.");
        let Py::Str(text) = minimum else {
            return Err(invalid_rate());
        };
        let text = text.as_str().ok_or_else(invalid_rate)?;
        if text.chars().count() > 40 {
            return Err(invalid_rate());
        }
        let floor: Decimal = kassiber_pytext::strip(text)
            .parse()
            .map_err(|_| invalid_rate())?;
        if floor.is_negative() {
            return Err(invalid_rate());
        }
        let rate = right["totals"]["final_fee_rate_sat_vb"].as_str();
        check(
            "minimum_final_fee_rate",
            rate.map(|r| decimal(r) >= floor),
            vec![],
        );
    }
    let statuses: Vec<String> = checks
        .iter()
        .map(|c| match kassiber_envelope::get(c, "status") {
            Some(Py::Str(v)) => v.as_str().unwrap_or("").to_owned(),
            _ => String::new(),
        })
        .collect();
    let status = if statuses.iter().any(|v| v == "failed") {
        "failed"
    } else if statuses.iter().any(|v| v == "unavailable") {
        "incomplete"
    } else {
        "checks_passed"
    };
    let contribution = supplied.then(|| {
        receivers
            .iter()
            .map(|i| {
                facts[*i]["amount_msat"]
                    .as_str()
                    .unwrap()
                    .parse::<u128>()
                    .unwrap()
            })
            .sum::<u128>()
    });
    Ok(dict([
        ("protocol", s("bip78")),
        ("status", s(status)),
        ("checks", Py::List(checks)),
        ("additional_input_count", num(receivers.len())),
        (
            "receiver_contribution_msat",
            contribution.map_or(Py::Null, string),
        ),
        ("negotiation_verified", Py::Bool(false)),
        ("safe_to_sign", Py::Null),
        (
            "limitations",
            Py::List(
                [
                    "local_supplied_psbts_only",
                    "signature_checks_are_not_full_consensus_script_execution",
                    "ownership_unspentness_and_negotiation_not_verified",
                    "final_feerate_requires_signed_proposal",
                ]
                .map(s)
                .to_vec(),
            ),
        ),
    ]))
}
