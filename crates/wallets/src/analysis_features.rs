//! Closed public structure shared by stored observations and PSBT proposals.
//! No keys, scripts, witnesses or free-text cache metadata leave this module.
//! Encoding observations never verify signatures or establish ownership.
use bitcoin::hashes::{hash160, sha256, Hash};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::str::FromStr;

mod normalize;
mod rules;
pub use normalize::normalize;
pub use rules::evaluate;

pub const FEATURE_VERSION: u32 = 1;
pub const RULE_VERSION: u32 = 1;
pub const PERSISTED_FEATURE_KEY: &str = "chain_analysis_features";

fn bytes(value: &Value) -> Option<Vec<u8>> {
    let text = value.as_str()?;
    if text.len() > 200_000 {
        return None;
    }
    let mut chars = text.bytes().peekable();
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        if matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 11 | 12) {
            continue;
        }
        let high = (c as char).to_digit(16)?;
        let low = (chars.next()? as char).to_digit(16)?;
        out.push((high * 16 + low) as u8);
    }
    Some(out)
}

pub(crate) fn kind(script: Option<&[u8]>) -> &'static str {
    let Some(s) = script else { return "unknown" };
    if s.len() == 25 && s.starts_with(&[0x76, 0xa9, 0x14]) && s.ends_with(&[0x88, 0xac]) {
        "p2pkh"
    } else if s.len() == 23 && s.starts_with(&[0xa9, 0x14]) && s.ends_with(&[0x87]) {
        "p2sh"
    } else if s.len() == 22 && s.starts_with(&[0, 0x14]) {
        "p2wpkh"
    } else if s.len() == 34 && s.starts_with(&[0, 0x20]) {
        "p2wsh"
    } else if s.len() == 34 && s.starts_with(&[0x51, 0x20]) {
        "p2tr"
    } else if s.starts_with(&[0x6a]) {
        "op_return"
    } else if matches!(s.len(), 35 | 67) && s[0] as usize == s.len() - 2 && s.last() == Some(&0xac)
    {
        "p2pk"
    } else if (4..=42).contains(&s.len())
        && (s[0] == 0 || (0x51..=0x60).contains(&s[0]))
        && s[1] as usize == s.len() - 2
    {
        "witness_unknown"
    } else {
        "other"
    }
}

pub fn script_type(value: &Value) -> &'static str {
    kind(bytes(value).as_deref())
}

pub fn signature_encoding(signature: &[u8], schnorr: bool) -> Option<Value> {
    let v = signature;
    if schnorr {
        if !matches!(v.len(), 64 | 65)
            || v.len() == 65 && ![1, 2, 3, 0x81, 0x82, 0x83].contains(&v[64])
        {
            return None;
        }
        return Some(
            json!({"algorithm":"schnorr","sighash":if v.len()==65 { v[64] } else { 0 },"low_r":null,"low_s":null,"verified":false}),
        );
    }
    if !(9..=73).contains(&v.len()) || v[0] != 0x30 || v[1] as usize != v.len() - 3 {
        return None;
    }
    let lr = v[3] as usize;
    if v[2] != 2 || lr == 0 || 5 + lr >= v.len() || v[4 + lr] != 2 {
        return None;
    }
    let ls = v[5 + lr] as usize;
    if ls == 0 || lr + ls + 7 != v.len() {
        return None;
    }
    let r = &v[4..4 + lr];
    let s = &v[6 + lr..v.len() - 1];
    if [r, s]
        .iter()
        .any(|n| n[0] & 0x80 != 0 || n.len() > 1 && n[0] == 0 && n[1] & 0x80 == 0)
    {
        return None;
    }
    let n = BigInt::parse_bytes(
        b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141",
        16,
    )
    .unwrap();
    let r = BigInt::from_bytes_be(kassiber_pyjson::BigInt::from(1).sign(), r);
    let s = BigInt::from_bytes_be(kassiber_pyjson::BigInt::from(1).sign(), s);
    if r <= BigInt::from(0) || r >= n || s <= BigInt::from(0) || s >= n {
        return None;
    }
    Some(
        json!({"algorithm":"ecdsa","sighash":v[v.len()-1],"low_r":r < (BigInt::from(1)<<255),"low_s":s <= n/2,"verified":false}),
    )
}

pub(crate) fn pushes(script: &[u8]) -> Option<Vec<&[u8]>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < script.len() {
        let opcode = script[pos];
        pos += 1;
        let size = if opcode <= 75 {
            opcode as usize
        } else if (76..=78).contains(&opcode) {
            let width = 1 << (opcode - 76);
            let data = script.get(pos..pos + width)?;
            pos += width;
            data.iter()
                .enumerate()
                .map(|(i, b)| (*b as usize) << (8 * i))
                .sum()
        } else {
            return None;
        };
        out.push(script.get(pos..pos.checked_add(size)?)?);
        pos += size;
    }
    Some(out)
}

fn multisig(s: &[u8]) -> bool {
    if s.len() < 4
        || !(0x51..=0x60).contains(&s[0])
        || !(0x51..=0x60).contains(&s[s.len() - 2])
        || s[s.len() - 1] != 0xae
    {
        return false;
    }
    pushes(&s[1..s.len() - 2]).is_some_and(|keys| {
        !keys.is_empty()
            && keys.len() == (s[s.len() - 2] - 0x50) as usize
            && s[0] <= s[s.len() - 2]
            && keys
                .iter()
                .all(|k| matches!(k.len(), 33 | 65) && [2, 3, 4].contains(&k[0]))
    })
}

fn input_script(v: &Value) -> &Value {
    output_script(&v["prevout"])
}
fn output_script(v: &Value) -> &Value {
    for k in ["scriptpubkey", "script_hex"] {
        if !v[k].is_null() {
            return &v[k];
        }
    }
    let nested = &v["scriptPubKey"];
    if nested.is_object() {
        if crate::descriptors::truthy(&nested["hex"]) {
            &nested["hex"]
        } else {
            &nested["scriptpubkey"]
        }
    } else if nested.is_string() {
        nested
    } else {
        &Value::Null
    }
}

fn input_signatures(entry: &Value) -> Vec<Value> {
    let mut script = bytes(input_script(entry));
    let mut k = kind(script.as_deref());
    let sig = entry.get("scriptsig").unwrap_or(&entry["scriptSig"]);
    let sig = if sig.is_object() { &sig["hex"] } else { sig };
    let sig_bytes = bytes(sig);
    let pushed = sig_bytes.as_deref().and_then(pushes);
    let witness: Option<Vec<_>> = entry
        .get("witness")
        .unwrap_or(&entry["txinwitness"])
        .as_array()
        .and_then(|v| v.iter().map(bytes).collect());
    let mut candidates: Vec<&[u8]> = Vec::new();
    let mut schnorr = false;
    if k == "p2sh" {
        if let Some(p) = pushed.as_ref().filter(|p| !p.is_empty()) {
            let redeem = p[p.len() - 1];
            if hash160::Hash::hash(redeem).as_byte_array() != &script.as_ref().unwrap()[2..22] {
                return Vec::new();
            }
            let nested_kind = kind(Some(redeem));
            if matches!(nested_kind, "p2wpkh" | "p2wsh") && p.len() == 1 {
                script = Some(redeem.to_vec());
                k = nested_kind;
            } else if multisig(redeem) && p[0].is_empty() {
                candidates = p[1..p.len() - 1].to_vec();
            }
        }
    }
    let witness = witness.as_deref().unwrap_or(&[]);
    let pushed = pushed.as_deref().unwrap_or(&[]);
    let script = script.as_deref().unwrap_or(&[]);
    if k == "p2wpkh"
        && witness.len() == 2
        && hash160::Hash::hash(&witness[1]).as_byte_array() == &script[2..]
    {
        candidates = vec![&witness[0]];
    } else if k == "p2wsh"
        && witness.len() >= 3
        && witness[0].is_empty()
        && sha256::Hash::hash(&witness[witness.len() - 1]).as_byte_array() == &script[2..]
        && multisig(&witness[witness.len() - 1])
    {
        candidates = witness[1..witness.len() - 1]
            .iter()
            .map(Vec::as_slice)
            .collect();
    } else if k == "p2pkh"
        && pushed.len() == 2
        && hash160::Hash::hash(pushed[1]).as_byte_array() == &script[3..23]
    {
        candidates = vec![pushed[0]];
    } else if k == "p2pk" && pushed.len() == 1 {
        candidates = pushed.to_vec();
    } else if k == "p2tr" && !witness.is_empty() {
        let stack = if witness.len() >= 2 && witness[witness.len() - 1].starts_with(&[0x50]) {
            &witness[..witness.len() - 1]
        } else {
            witness
        };
        if stack.len() == 1 {
            candidates = vec![&stack[0]];
            schnorr = true;
        }
    }
    candidates
        .into_iter()
        .filter_map(|v| signature_encoding(v, schnorr))
        .collect()
}

fn integer(v: &Value, maximum: u64) -> Option<u64> {
    v.as_u64().filter(|n| *n <= maximum)
}
fn big(v: &Value) -> Option<BigInt> {
    v.as_number().and_then(|n| n.as_str().parse().ok())
}
fn number(n: &BigInt) -> Value {
    Value::Number(n.to_string().parse().unwrap())
}

pub(crate) fn value_sats(v: &Value) -> Option<BigInt> {
    let explicit = !v["value_sats"].is_null();
    let v = if explicit {
        &v["value_sats"]
    } else {
        &v["value"]
    };
    if v.is_null() || v.is_boolean() || v.as_str() == Some("") {
        return None;
    }
    let integer = explicit || big(v).is_some() || v.as_str().is_some_and(|s| !s.contains('.'));
    if integer {
        if let Some(n) = big(v) {
            return Some(n);
        }
        if let Some(s) = v.as_str() {
            return kassiber_pytext::parse_int(s).ok();
        }
        return crate::scan::integer(v);
    }
    let text = v
        .as_str()
        .map(str::to_owned)
        .or_else(|| v.as_f64().map(kassiber_pyjson::float_repr))
        .unwrap_or_else(|| v.to_string());
    let decimal = Decimal::from_str(kassiber_pytext::strip(&text)).ok()?;
    let decimal = decimal
        .mul(&Decimal::from(100_000_000_u64), &Context::RP2)
        .ok()?;
    decimal
        .to_fixed_places(0, Rounding::HalfEven)
        .ok()?
        .parse()
        .ok()
}

fn input_value(v: &Value) -> Option<BigInt> {
    value_sats(&v["prevout"]).or_else(|| value_sats(v))
}

struct Snapshot {
    rows: Vec<Value>,
    source: String,
    subject: Option<String>,
}
impl Snapshot {
    fn add(
        &mut self,
        code: &str,
        value: Value,
        available: bool,
        partial: bool,
        assumptions: &[&str],
    ) {
        self.rows.push(json!({"code":code,"value":value,"availability":if available {if partial {"partial"} else {"observed"}} else {"unavailable"},"source":self.source,"confidence":if available {"observed"} else {"unknown"},"evidence":[{"source":self.source,"subject_id":self.subject,"field":code}],"assumptions":assumptions}));
    }
    fn finish(self) -> Value {
        json!({"schema_version":1,"extractor_version":FEATURE_VERSION,"source":self.source,"subject_id":self.subject,"features":self.rows})
    }
}

pub fn extract(raw: &Value, source: &str, subject: Option<&str>, signatures: &[Value]) -> Value {
    let mut s = Snapshot {
        rows: Vec::new(),
        source: source.into(),
        subject: subject.map(str::to_owned),
    };
    let vin = raw["vin"].as_array();
    let vout = raw["vout"].as_array();
    let inputs: Vec<_> = vin
        .into_iter()
        .flatten()
        .filter(|r| r.is_object())
        .collect();
    let outputs: Vec<_> = vout
        .into_iter()
        .flatten()
        .filter(|r| r.is_object())
        .collect();
    let ic = vin.is_some_and(|v| v.len() == inputs.len());
    let oc = vout.is_some_and(|v| v.len() == outputs.len());
    let mut version = integer(&raw["version"], u32::MAX.into()).map(|n| n as u32 as i32 as i64);
    if raw["version"]
        .as_i64()
        .is_some_and(|n| (i32::MIN as i64..0).contains(&n))
    {
        version = raw["version"].as_i64();
    }
    let locktime = integer(&raw["locktime"], u32::MAX.into());
    s.add(
        "transaction_version",
        json!(version),
        version.is_some(),
        false,
        &[],
    );
    s.add(
        "cardinality",
        json!({"inputs":inputs.len(),"outputs":outputs.len()}),
        ic && oc,
        false,
        &[],
    );
    let seq: Vec<_> = inputs
        .iter()
        .map(|r| integer(&r["sequence"], u32::MAX.into()))
        .collect();
    let sc = !inputs.is_empty() && ic && seq.iter().all(Option::is_some);
    let rbf = seq.iter().flatten().any(|n| *n < 0xffff_fffe);
    let relative: Vec<_> = seq.iter().enumerate().filter_map(|(i,n)| n.filter(|n| version.is_some_and(|v| v>=2) && n & (1<<31) == 0).map(|n| json!({"input_index":i,"unit":if n&(1<<22)!=0 {"seconds"} else {"blocks"},"minimum":(n&0xffff)*if n&(1<<22)!=0 {512} else {1}}))).collect();
    s.add("sequences", json!({"values":seq,"signals_bip125":if rbf || sc {Some(rbf)} else {None},"relative_locks":relative,"relative_lock_interpretation_available":version.is_some()}), seq.iter().any(Option::is_some), !sc, &["bip125_signal_is_not_mempool_acceptance_or_replaceability","relative_lock_maturity_requires_prevout_chain_context"]);
    let enabled = seq.iter().flatten().any(|n| *n != 0xffff_ffff);
    s.add("absolute_locktime", json!({"value":locktime,"kind":match locktime {Some(0)=>"none",Some(n) if n<500_000_000=>"height",Some(_)=>"time",None=>"unknown"},"enabled":if enabled || sc {Some(enabled)} else {None}}), locktime.is_some(), false, &["locktime_maturity_requires_chain_tip_and_median_time_past"]);
    for (side, rows, complete) in [("input", &inputs, ic), ("output", &outputs, oc)] {
        let kinds: Vec<_> = rows
            .iter()
            .map(|r| {
                script_type(if side == "input" {
                    input_script(r)
                } else {
                    output_script(r)
                })
            })
            .collect();
        let known = kinds.iter().filter(|k| **k != "unknown").count();
        let mut counts = BTreeMap::new();
        for k in &kinds {
            *counts.entry(k).or_insert(0_u64) += 1;
        }
        s.add(
            &format!("{side}_script_types"),
            json!({"counts":counts,"by_index":kinds}),
            known > 0,
            !complete || known != rows.len(),
            &[],
        );
    }
    let mut observed = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        for mut sig in input_signatures(input) {
            sig["input_index"] = json!(i);
            observed.push(sig);
        }
    }
    for row in signatures {
        if matches!(row["algorithm"].as_str(), Some("ecdsa" | "schnorr"))
            && integer(&row["sighash"], 255).is_some()
        {
            observed.push(Value::Object(
                [
                    "algorithm",
                    "sighash",
                    "low_r",
                    "low_s",
                    "input_index",
                    "verified",
                ]
                .map(|k| (k.into(), row[k].clone()))
                .into_iter()
                .collect(),
            ));
        }
    }
    s.add(
        "signature_encodings",
        json!({"observations":observed,"count":observed.len()}),
        !observed.is_empty(),
        false,
        &[
            "encoding_observation_does_not_verify_signature_or_identify_wallet",
            "unrecognized_script_positions_are_not_scanned",
        ],
    );
    let shapes: Vec<_> = inputs
        .iter()
        .map(|r| {
            r.get("witness")
                .unwrap_or(&r["txinwitness"])
                .as_array()
                .map(Vec::len)
        })
        .collect();
    s.add(
        "witness_shapes",
        json!(shapes),
        shapes.iter().any(Option::is_some),
        shapes.iter().any(Option::is_none),
        &[],
    );
    let values: Vec<_> = outputs.iter().map(|r| value_sats(r)).collect();
    let known: Vec<_> = values
        .iter()
        .flatten()
        .filter(|v| **v >= BigInt::from(0))
        .collect();
    let mut counts = BTreeMap::new();
    for n in &known {
        *counts.entry(*n).or_insert(0_u64) += 1;
    }
    let groups: Vec<_> = counts
        .into_iter()
        .filter(|(n, c)| **n > BigInt::from(0) && *c >= 2)
        .map(|(n, c)| json!({"amount_msat":(n*1000_u32).to_string(),"count":c}))
        .collect();
    s.add("output_value_pattern", json!({"equal_groups":groups,"round_1000_sat_count":known.iter().filter(|n| ***n>BigInt::from(0) && (**n % 1000_u32)==BigInt::from(0)).count()}), !known.is_empty(), !oc || known.len()!=outputs.len(), &[]);
    let scripts: Vec<_> = outputs.iter().map(|r| bytes(output_script(r))).collect();
    let oa = outputs.len() >= 2
        && oc
        && known.len() == outputs.len()
        && scripts.iter().all(Option::is_some);
    let ordered = oa.then(|| {
        values
            .iter()
            .zip(&scripts)
            .zip(values.iter().zip(&scripts).skip(1))
            .all(|(a, b)| a <= b)
    });
    s.add(
        "output_ordering",
        json!({"bip69_value_script_order":ordered}),
        oa,
        false,
        &["matching_sort_order_can_occur_by_chance"],
    );
    let iv: Vec<_> = inputs.iter().map(|r| input_value(r)).collect();
    let vsize = integer(&raw["vsize"], u32::MAX.into());
    let fa = !inputs.is_empty()
        && !outputs.is_empty()
        && ic
        && oc
        && iv.iter().chain(&values).all(Option::is_some);
    let fee =
        fa.then(|| iv.iter().flatten().sum::<BigInt>() - values.iter().flatten().sum::<BigInt>());
    let fa =
        fa && fee.as_ref().is_some_and(|n| *n >= BigInt::from(0)) && vsize.is_some_and(|n| n > 0);
    s.add("fee_rate", json!({"fee_sats":if fa {number(fee.as_ref().unwrap())} else {Value::Null},"vsize":if fa {vsize} else {None}}), fa, false, &["observed_transaction_values_only","rounded_rate_does_not_identify_wallet"]);
    s.finish()
}

#[cfg(test)]
mod tests;
