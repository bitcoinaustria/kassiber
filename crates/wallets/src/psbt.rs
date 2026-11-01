//! Bounded, transient PSBT v0/v2 reads. No signing or wallet authority.
use crate::analysis_features::{evaluate, extract, signature_encoding};
use bitcoin::hashes::{hash160, sha256, Hash};
use kassiber_envelope::{dict, s, AppError};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const MAX_PSBT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LEGS: u64 = 4096;
const MAX_MONEY: u64 = 21_000_000 * 100_000_000;
type Result<T> = std::result::Result<T, AppError>;
type Map<'a> = Vec<(&'a [u8], &'a [u8])>;

mod compare;
mod order;
pub use compare::{analyze_parts, analyze_payload, compare, decode, Payload};

fn invalid(reason: &str) -> AppError {
    AppError::new(
        "chain_analysis_invalid_psbt",
        "The PSBT cannot be analyzed safely.",
    )
    .with_details(dict([("reason", s(reason))]))
}
fn at(reason: &str, index: usize) -> AppError {
    invalid(reason).with_details(dict([
        ("reason", s(reason)),
        ("input_index", kassiber_pyjson::Value::Int(index.into())),
    ]))
}

struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }
    fn take(&mut self, size: u64) -> Result<&'a [u8]> {
        let size = usize::try_from(size).map_err(|_| invalid("truncated_encoding"))?;
        let end = self
            .offset
            .checked_add(size)
            .ok_or_else(|| invalid("truncated_encoding"))?;
        let out = self
            .data
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated_encoding"))?;
        self.offset = end;
        Ok(out)
    }
    fn number(&mut self, width: u64) -> Result<u64> {
        let bytes = self.take(width)?;
        Ok(bytes
            .iter()
            .enumerate()
            .fold(0, |n, (i, b)| n | u64::from(*b) << (8 * i)))
    }
    fn compact(&mut self) -> Result<u64> {
        self.compact_number(true)
    }
    fn compact_number(&mut self, canonical: bool) -> Result<u64> {
        let marker = self.number(1)?;
        let (width, minimum) = match marker {
            253 => (2, 253),
            254 => (4, 65536),
            255 => (8, 1u64 << 32),
            _ => return Ok(marker),
        };
        let n = self.number(width)?;
        if canonical && n < minimum {
            return Err(invalid("noncanonical_compact_size"));
        }
        Ok(n)
    }
    fn variable(&mut self) -> Result<&'a [u8]> {
        let n = self.compact()?;
        self.take(n)
    }
    fn legacy_variable(&mut self) -> Result<&'a [u8]> {
        let n = self.compact_number(false)?;
        self.take(n)
    }
    fn done(&self) -> bool {
        self.offset == self.data.len()
    }
    fn mapping(&mut self) -> Result<Map<'a>> {
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        loop {
            let n = self.compact()?;
            if n == 0 {
                return Ok(entries);
            }
            let key = self.take(n)?;
            Reader::new(key).compact()?;
            if !seen.insert(key) {
                return Err(invalid("duplicate_map_key"));
            }
            if entries.len() >= 10_000 {
                return Err(invalid("map_entry_limit"));
            }
            entries.push((key, self.variable()?));
        }
    }
}

// embit's BytesIO decoder leaks RuntimeError at empty compact-size reads.
// Preserve that refusal as a native internal error, without loosening the
// canonical decoder. This scanner allocates nothing and every loop consumes
// bytes or returns a refusal, even for adversarial 64-bit counts.
fn embedded_preflight(raw: &[u8], scope: &str, reason: &str) -> Result<()> {
    struct Loose<'a> {
        data: &'a [u8],
        offset: usize,
    }
    impl<'a> Loose<'a> {
        fn take(&mut self, n: u64) -> &'a [u8] {
            let n = usize::try_from(n).unwrap_or(usize::MAX);
            let end = self.offset.saturating_add(n).min(self.data.len());
            let value = &self.data[self.offset..end];
            self.offset = end;
            value
        }
        fn compact(&mut self) -> Result<u64> {
            let Some(&n) = self.take(1).first() else {
                return Err(AppError::new(
                    "internal_error",
                    "Can't read one byte from the stream",
                ));
            };
            let width = match n {
                253 => 2,
                254 => 4,
                255 => 8,
                _ => return Ok(u64::from(n)),
            };
            Ok(self
                .take(width)
                .iter()
                .enumerate()
                .fold(0, |a, (i, b)| a | u64::from(*b) << (8 * i)))
        }
        fn script(&mut self, reason: &str) -> Result<()> {
            let n = self.compact()?;
            if self.take(n).len() as u64 != n {
                return Err(invalid(reason));
            }
            Ok(())
        }
        fn witness(&mut self) -> Result<()> {
            let n = self.compact()?;
            for _ in 0..n {
                let size = self.compact()?;
                self.take(size);
            }
            Ok(())
        }
    }
    let mut r = Loose {
        data: raw,
        offset: 0,
    };
    match scope {
        "witness" => r.witness()?,
        "output" => {
            r.take(8);
            r.script(reason)?;
        }
        _ => {
            r.take(4);
            let mut n = r.compact()?;
            let segwit = n == 0;
            if segwit {
                if r.take(1) != [1] {
                    return Err(invalid(reason));
                }
                n = r.compact()?;
            }
            for _ in 0..n {
                r.take(32);
                r.take(4);
                r.script(reason)?;
                r.take(4);
            }
            let outputs = r.compact()?;
            for _ in 0..outputs {
                r.take(8);
                r.script(reason)?;
            }
            if segwit {
                for _ in 0..n {
                    r.witness()?;
                }
            }
        }
    }
    Ok(())
}
fn field<'a>(map: &Map<'a>, key: u8) -> Option<&'a [u8]> {
    map.iter().find(|(k, _)| *k == [key]).map(|(_, v)| *v)
}
fn number(map: &Map<'_>, key: u8, width: usize, required: bool) -> Result<Option<u64>> {
    let Some(raw) = field(map, key) else {
        return if required {
            Err(invalid("missing_required_field"))
        } else {
            Ok(None)
        };
    };
    if raw.len() != width {
        return Err(invalid("invalid_field_length"));
    }
    Ok(Some(Reader::new(raw).number(width as u64)?))
}
fn count(map: &Map<'_>, key: u8) -> Result<u64> {
    let raw = field(map, key).ok_or_else(|| invalid("missing_required_field"))?;
    let mut reader = Reader::new(raw);
    let n = reader.compact()?;
    if !reader.done() || n > MAX_LEGS {
        return Err(invalid("invalid_or_excessive_leg_count"));
    }
    Ok(n)
}
fn typed_map(map: &Map<'_>, scope: &str, version: u64) -> Result<()> {
    let singleton: &[u64] = match scope {
        "global" => &[0, 2, 3, 4, 5, 6, 0xfb],
        "input" => &[
            0, 1, 3, 4, 5, 7, 8, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x17, 0x18,
        ],
        _ => &[0, 1, 3, 4, 5, 6],
    };
    let v2_only: &[u64] = match scope {
        "global" => &[2, 3, 4, 5, 6],
        "input" => &[0x0e, 0x0f, 0x10, 0x11, 0x12],
        _ => &[3, 4],
    };
    for (key, value) in map {
        let kind = Reader::new(key).compact()?;
        if singleton.contains(&kind) && key.len() != 1 {
            return Err(invalid("unexpected_key_data"));
        }
        if version == 0 && v2_only.contains(&kind) || version == 2 && scope == "global" && kind == 0
        {
            return Err(invalid("field_forbidden_in_psbt_version"));
        }
        if (scope == "input" && [2, 6].contains(&kind) || scope == "output" && kind == 2)
            && (!matches!(key.get(1), Some(2..=4))
                || bitcoin::PublicKey::from_slice(&key[1..]).is_err())
        {
            return Err(invalid("invalid_public_key_encoding"));
        }
        if (scope == "input" && kind == 6 || scope == "output" && kind == 2)
            && (value.len() < 4 || value.len() % 4 != 0)
        {
            return Err(invalid("invalid_derivation_encoding"));
        }
    }
    Ok(())
}

fn base64(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut count, mut padding) = (0u32, 0, 0);
    let mut ended = false;
    let mut padded = false;
    for &b in text {
        if ended {
            return None;
        }
        if b == b'=' {
            padded = true;
            if count < 2 {
                if count != 0 || out.is_empty() {
                    return None;
                }
                continue;
            }
            padding += 1;
            if count + padding == 4 {
                ended = true;
            }
            continue;
        }
        if padded {
            return None;
        }
        let n = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = bits << 6 | u32::from(n);
        count += 1;
        if count >= 2 {
            out.push((bits >> (6 * count - 8 * (count - 1))) as u8);
        }
        if count == 4 {
            count = 0;
            bits = 0;
        }
    }
    (ended || count == 0).then_some(out)
}

/// The text path uses CPython's whitespace and strict Base64 rules.
pub fn text_bytes(text: &str) -> Result<Vec<u8>> {
    if text.chars().count() > MAX_PSBT_BYTES * 3 {
        return Err(invalid("payload_limit"));
    }
    let compact: String = text
        .chars()
        .filter(|c| !kassiber_pytext::is_space(*c as u32))
        .collect();
    let decoded = if compact.starts_with("70736274ff") {
        use bitcoin::hex::FromHex;
        Vec::<u8>::from_hex(&compact).ok()
    } else {
        base64(compact.as_bytes())
    };
    let data = decoded.ok_or_else(|| invalid("invalid_base64_or_hex"))?;
    if data.len() > MAX_PSBT_BYTES {
        return Err(invalid("payload_limit"));
    }
    Ok(data)
}
fn payload(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.starts_with(b"psbt\xff") {
        if raw.len() > MAX_PSBT_BYTES {
            return Err(invalid("payload_limit"));
        }
        return Ok(raw.to_vec());
    }
    if raw.len() > MAX_PSBT_BYTES * 3 {
        return Err(invalid("payload_limit"));
    }
    if !raw.is_ascii() {
        return Err(invalid("invalid_binary_or_text_encoding"));
    }
    text_bytes(std::str::from_utf8(raw).unwrap())
}

struct Input<'a> {
    txid: &'a [u8],
    vout: u64,
    script: &'a [u8],
    sequence: u64,
    witness: Vec<&'a [u8]>,
}
struct Output<'a> {
    amount: u64,
    script: &'a [u8],
}
struct Transaction<'a> {
    version: u64,
    locktime: u64,
    inputs: Vec<Input<'a>>,
    outputs: Vec<Output<'a>>,
}
fn output<'a>(r: &mut Reader<'a>) -> Result<Output<'a>> {
    Ok(Output {
        amount: r.number(8)?,
        script: r.variable()?,
    })
}
fn witness(raw: &[u8]) -> Result<Vec<&[u8]>> {
    embedded_preflight(raw, "witness", "invalid_final_witness")?;
    let mut r = Reader::new(raw);
    let values = read_witness(&mut r).map_err(|_| invalid("invalid_final_witness"))?;
    if !r.done() {
        return Err(invalid("invalid_final_witness"));
    }
    Ok(values)
}
fn read_witness<'a>(r: &mut Reader<'a>) -> Result<Vec<&'a [u8]>> {
    let n = r.compact()?;
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(r.variable()?);
    }
    Ok(out)
}
fn transaction<'a>(r: &mut Reader<'a>, unsigned: bool) -> Result<Transaction<'a>> {
    let version = r.number(4)?;
    let mut ni = r.compact()?;
    let segwit = !unsigned && ni == 0;
    if segwit {
        if r.number(1)? != 1 {
            return Err(invalid("invalid_non_witness_utxo"));
        }
        ni = r.compact()?;
    }
    if unsigned && ni > MAX_LEGS {
        return Err(invalid("invalid_or_excessive_leg_count"));
    }
    let mut inputs = Vec::new();
    for _ in 0..ni {
        let start = r.offset;
        let item = Input {
            txid: r.take(32)?,
            vout: r.number(4)?,
            script: if unsigned {
                r.legacy_variable()?
            } else {
                r.variable()?
            },
            sequence: r.number(4)?,
            witness: Vec::new(),
        };
        if unsigned && r.offset - start != 40 + compact_width(item.script.len()) + item.script.len()
        {
            return Err(invalid("invalid_unsigned_transaction"));
        }
        inputs.push(item);
    }
    let no = r.compact()?;
    if unsigned && no > MAX_LEGS {
        return Err(invalid("invalid_or_excessive_leg_count"));
    }
    let mut outputs = Vec::new();
    for _ in 0..no {
        let start = r.offset;
        let item = Output {
            amount: r.number(8)?,
            script: if unsigned {
                r.legacy_variable()?
            } else {
                r.variable()?
            },
        };
        if unsigned && r.offset - start != 8 + compact_width(item.script.len()) + item.script.len()
        {
            return Err(invalid("invalid_unsigned_transaction"));
        }
        outputs.push(item);
    }
    if segwit {
        for input in &mut inputs {
            input.witness = read_witness(r)?;
        }
        if inputs.iter().all(|v| v.witness.is_empty()) {
            return Err(invalid("invalid_non_witness_utxo"));
        }
    }
    let locktime = r.number(4)?;
    if !r.done() {
        return Err(invalid(if unsigned {
            "invalid_unsigned_transaction"
        } else {
            "invalid_non_witness_utxo"
        }));
    }
    Ok(Transaction {
        version,
        locktime,
        inputs,
        outputs,
    })
}
fn compact_width(n: usize) -> usize {
    if n < 253 {
        1
    } else if n <= 0xffff {
        3
    } else if n <= 0xffff_ffff {
        5
    } else {
        9
    }
}
fn compact(n: u64, out: &mut Vec<u8>) {
    let width = if n < 253 {
        out.push(n as u8);
        return;
    } else if n <= 0xffff {
        out.push(253);
        2
    } else if n <= 0xffff_ffff {
        out.push(254);
        4
    } else {
        out.push(255);
        8
    };
    out.extend_from_slice(&n.to_le_bytes()[..width]);
}
fn variable(data: &[u8], out: &mut Vec<u8>) {
    compact(data.len() as u64, out);
    out.extend_from_slice(data);
}
fn stripped(tx: &Transaction<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(tx.version as u32).to_le_bytes());
    compact(tx.inputs.len() as u64, &mut out);
    for i in &tx.inputs {
        out.extend_from_slice(i.txid);
        out.extend_from_slice(&(i.vout as u32).to_le_bytes());
        variable(i.script, &mut out);
        out.extend_from_slice(&(i.sequence as u32).to_le_bytes());
    }
    compact(tx.outputs.len() as u64, &mut out);
    for o in &tx.outputs {
        out.extend_from_slice(&o.amount.to_le_bytes());
        variable(o.script, &mut out);
    }
    out.extend_from_slice(&(tx.locktime as u32).to_le_bytes());
    out
}
fn in_range(outputs: &[Output<'_>]) -> bool {
    outputs.iter().all(|o| o.amount <= MAX_MONEY)
        && outputs.iter().map(|o| u128::from(o.amount)).sum::<u128>() <= u128::from(MAX_MONEY)
}
fn hex(raw: &[u8]) -> String {
    use bitcoin::hex::DisplayHex;
    raw.to_lower_hex_string()
}
fn txid(raw: &[u8]) -> String {
    hex(&raw.iter().rev().copied().collect::<Vec<_>>())
}
fn kind(script: &[u8]) -> &'static str {
    crate::analysis_features::kind(Some(script))
}

struct Parsed<'a> {
    version: u64,
    tx: Transaction<'a>,
    globals: Map<'a>,
    inputs: Vec<Map<'a>>,
    outputs: Vec<Map<'a>>,
}
fn parse(data: &[u8]) -> Result<Parsed<'_>> {
    let mut r = Reader::new(data);
    if r.take(5)? != b"psbt\xff" {
        return Err(invalid("invalid_magic"));
    }
    let globals = r.mapping()?;
    let version = number(&globals, 0xfb, 4, false)?.unwrap_or(0);
    if ![0, 2].contains(&version) {
        return Err(invalid("unsupported_psbt_version"));
    }
    typed_map(&globals, "global", version)?;
    let (mut tx, ni, no) = if version == 0 {
        let raw = field(&globals, 0).ok_or_else(|| invalid("missing_unsigned_transaction"))?;
        let tx = transaction(&mut Reader::new(raw), true)?;
        if tx.inputs.iter().any(|i| !i.script.is_empty()) {
            return Err(invalid("unsigned_transaction_contains_scripts_or_witness"));
        }
        let (ni, no) = (tx.inputs.len() as u64, tx.outputs.len() as u64);
        (tx, ni, no)
    } else {
        let v = number(&globals, 2, 4, true)?.unwrap();
        let (ni, no) = (count(&globals, 4)?, count(&globals, 5)?);
        let locktime = number(&globals, 3, 4, false)?.unwrap_or(0);
        number(&globals, 6, 1, false)?;
        (
            Transaction {
                version: v,
                locktime,
                inputs: vec![],
                outputs: vec![],
            },
            ni,
            no,
        )
    };
    let inputs = (0..ni).map(|_| r.mapping()).collect::<Result<Vec<_>>>()?;
    let outputs = (0..no).map(|_| r.mapping()).collect::<Result<Vec<_>>>()?;
    if !r.done() {
        return Err(invalid("trailing_bytes_or_extra_maps"));
    }
    for row in &inputs {
        typed_map(row, "input", version)?;
    }
    for row in &outputs {
        typed_map(row, "output", version)?;
    }
    if version == 2 {
        let (mut time, mut height, mut allow_time, mut allow_height) = (0, 0, true, true);
        for row in &inputs {
            let txid = field(row, 0x0e)
                .filter(|v| v.len() == 32)
                .ok_or_else(|| invalid("missing_or_invalid_previous_txid"))?;
            let vout = number(row, 0x0f, 4, true)?.unwrap();
            let sequence = number(row, 0x10, 4, false)?.unwrap_or(0xffff_ffff);
            tx.inputs.push(Input {
                txid,
                vout,
                sequence,
                script: &[],
                witness: vec![],
            });
            let t = number(row, 0x11, 4, false)?;
            let h = number(row, 0x12, 4, false)?;
            if t.is_some_and(|n| n < 500_000_000) || h.is_some_and(|n| n == 0 || n >= 500_000_000) {
                return Err(invalid("invalid_required_locktime"));
            }
            if t.is_some() || h.is_some() {
                allow_time &= t.is_some();
                allow_height &= h.is_some();
            }
            time = time.max(t.unwrap_or(0));
            height = height.max(h.unwrap_or(0));
        }
        if !allow_time && !allow_height {
            return Err(invalid("incompatible_required_locktime_types"));
        }
        tx.locktime = if allow_height && height != 0 {
            height
        } else if time != 0 {
            time
        } else {
            tx.locktime
        };
        for row in &outputs {
            let amount = number(row, 3, 8, true)?.unwrap();
            let script = field(row, 4).ok_or_else(|| invalid("missing_output_script"))?;
            tx.outputs.push(Output { amount, script });
        }
    }
    if !in_range(&tx.outputs) {
        return Err(invalid("output_amount_out_of_range"));
    }
    let points: BTreeSet<_> = tx.inputs.iter().map(|i| (i.txid, i.vout)).collect();
    if points.len() != tx.inputs.len() {
        return Err(invalid("duplicate_input_outpoint"));
    }
    Ok(Parsed {
        version,
        tx,
        globals,
        inputs,
        outputs,
    })
}
fn input_utxo<'a>(
    row: &Map<'a>,
    input: &Input<'_>,
    index: usize,
) -> Result<(Option<Output<'a>>, &'static str)> {
    let mut previous = None;
    if let Some(raw) = field(row, 0) {
        embedded_preflight(raw, "transaction", "invalid_non_witness_utxo")?;
        let mut tx = transaction(&mut Reader::new(raw), false)
            .map_err(|_| invalid("invalid_non_witness_utxo"))?;
        let bitcoin_tx: bitcoin::Transaction = bitcoin::consensus::deserialize(raw)
            .map_err(|_| invalid("invalid_non_witness_utxo"))?;
        if bitcoin_tx.compute_txid().as_byte_array() != input.txid {
            return Err(at("non_witness_utxo_txid_mismatch", index));
        }
        if input.vout >= tx.outputs.len() as u64 {
            return Err(at("non_witness_utxo_outpoint_out_of_range", index));
        }
        if !in_range(&tx.outputs) {
            return Err(at("previous_transaction_amount_out_of_range", index));
        }
        previous = Some(tx.outputs.remove(input.vout as usize));
    }
    let mut supplied = None;
    if let Some(raw) = field(row, 1) {
        embedded_preflight(raw, "output", "invalid_witness_utxo")?;
        let mut r = Reader::new(raw);
        let out = output(&mut r).map_err(|_| invalid("invalid_witness_utxo"))?;
        if !r.done() {
            return Err(invalid("invalid_witness_utxo"));
        }
        if out.amount > MAX_MONEY {
            return Err(at("input_amount_out_of_range", index));
        }
        supplied = Some(out);
    }
    if let (Some(a), Some(b)) = (&previous, &supplied) {
        if a.amount != b.amount || a.script != b.script {
            return Err(at("witness_non_witness_utxo_conflict", index));
        }
    }
    let evidence = if previous.is_some() {
        "previous_transaction_hash_verified"
    } else if supplied.is_some() {
        "supplied_witness_utxo"
    } else {
        "missing"
    };
    let utxo = previous.or(supplied);
    if let Some(out) = &utxo {
        let redeem = field(row, 4);
        if let Some(redeem) = redeem {
            if kind(out.script) != "p2sh"
                || hash160::Hash::hash(redeem).as_byte_array() != &out.script[2..22]
            {
                return Err(at("redeem_script_mismatch", index));
            }
        }
        let effective = redeem.unwrap_or(out.script);
        if let Some(script) = field(row, 5) {
            if kind(effective) != "p2wsh"
                || sha256::Hash::hash(script).as_byte_array() != &effective[2..]
            {
                return Err(at("witness_script_mismatch", index));
            }
        }
        if field(row, 1).is_some()
            && !["p2sh", "p2wpkh", "p2wsh", "p2tr", "witness_unknown"].contains(&kind(out.script))
        {
            return Err(at("witness_utxo_for_nonsegwit_script", index));
        }
        if field(row, 1).is_some()
            && redeem.is_some_and(|s| !["p2wpkh", "p2wsh"].contains(&kind(s)))
        {
            return Err(at("witness_utxo_for_nonsegwit_redeem_script", index));
        }
    }
    Ok((utxo, evidence))
}

/// Transient local inventory adapter. Scripts here must never reach AI/storage.
pub fn decode_structure(raw: &[u8]) -> Result<Value> {
    structure(&payload(raw)?)
}
pub fn decode_text_structure(text: &str) -> Result<Value> {
    structure(&text_bytes(text)?)
}
fn structure(data: &[u8]) -> Result<Value> {
    let p = parse(data)?;
    for (i, (input, row)) in p.tx.inputs.iter().zip(&p.inputs).enumerate() {
        input_utxo(row, input, i)?;
    }
    Ok(json!({"version":p.tx.version,"locktime":p.tx.locktime,
        "inputs":p.tx.inputs.iter().map(|i| json!({"prev_txid":txid(i.txid),"vout":i.vout,"sequence":i.sequence})).collect::<Vec<_>>(),
        "outputs":p.tx.outputs.iter().map(|o| json!({"value_msat":o.amount*1000,"script_key":hex(o.script),"is_op_return":o.script.starts_with(&[0x6a])})).collect::<Vec<_>>(),
        "unsigned_tx_clean":true,"signature_material_present":p.inputs.iter().any(|row| row.iter().any(|(k,_)| [2,7,8,0x13,0x14].contains(&k[0])))}))
}

/// Redacted shared analysis. Supplied UTXOs do not prove membership/unspentness.
pub fn analyze(raw: &[u8], network: &str, subject: &str) -> Result<Value> {
    analysis(&payload(raw)?, network, subject)
}
pub fn analyze_text(text: &str, network: &str, subject: &str) -> Result<Value> {
    analysis(&text_bytes(text)?, network, subject)
}
fn analysis(data: &[u8], network: &str, subject: &str) -> Result<Value> {
    let p = parse(data)?;
    if !["main", "test", "signet", "regtest"].contains(&network) {
        return Err(AppError::new(
            "chain_analysis_invalid_network",
            "Select an explicit Bitcoin network.",
        ));
    }
    let (mut inputs, mut raw_inputs, mut signatures, mut metadata) =
        (vec![], vec![], vec![], vec![]);
    let (mut total_in, mut known, mut all_final, mut final_scripts, mut final_witnesses) =
        (0u128, !p.tx.inputs.is_empty(), true, vec![], vec![]);
    for (i, (input, row)) in p.tx.inputs.iter().zip(&p.inputs).enumerate() {
        let (utxo, evidence) = input_utxo(row, input, i)?;
        let amount = utxo.as_ref().map(|u| u.amount * 1000);
        known &= amount.is_some();
        total_in += u128::from(amount.unwrap_or(0));
        inputs.push(json!({"output_id":format!("bitcoin:{network}:out:{}:{}",txid(input.txid),input.vout),"amount_msat":amount.map(|v| v.to_string()),"input_index":i,"sequence":input.sequence,"utxo_evidence":evidence}));
        let mut raw = json!({"txid":txid(input.txid),"vout":input.vout,"sequence":input.sequence,"prevout":utxo.as_ref().map_or(json!({}), |u| json!({"value_sats":u.amount,"scriptpubkey":hex(u.script)}))});
        if let Some(v) = field(row, 7) {
            raw["scriptsig"] = json!(hex(v));
        }
        let stacks = field(row, 8).map(witness).transpose()?;
        if let Some(v) = &stacks {
            raw["witness"] = json!(v.iter().map(|s| hex(s)).collect::<Vec<_>>());
        }
        raw_inputs.push(raw);
        let finalized = field(row, 7).is_some() || field(row, 8).is_some();
        all_final &= finalized;
        final_scripts.push(field(row, 7).unwrap_or(&[]));
        final_witnesses.push(stacks.unwrap_or_default());
        let sighash = number(row, 3, 4, false)?;
        let mut partial = 0;
        for (key, bytes) in row {
            if ![2, 0x13, 0x14].contains(&key[0]) {
                continue;
            }
            if key[0] == 2 && (!matches!(key.len(), 34 | 66) || ![2, 3, 4].contains(&key[1]))
                || key[0] == 0x13 && key.len() != 1
                || key[0] == 0x14 && key.len() != 65
            {
                return Err(at("invalid_partial_signature_key", i));
            }
            let mut observed = signature_encoding(bytes, key[0] != 2)
                .ok_or_else(|| at("invalid_partial_signature_encoding", i))?;
            if sighash.is_some_and(|s| observed["sighash"].as_u64() != Some(s)) {
                return Err(at("partial_signature_sighash_mismatch", i));
            }
            if !finalized {
                observed["input_index"] = json!(i);
                signatures.push(observed);
            }
            partial += 1;
        }
        metadata.push(json!({"input_index":i,"finalized":finalized,"partial_signature_count":partial,"derivation_metadata_present":row.iter().any(|(k,_)| [6,0x16].contains(&k[0])),"sighash_declared":sighash}));
    }
    let outputs:Vec<_> = p.tx.outputs.iter().enumerate().map(|(i,o)| json!({"output_id":format!("{subject}:out:{i}"),"output_index":i,"amount_msat":(o.amount*1000).to_string(),"script_type":kind(o.script)})).collect();
    let raw = json!({"version":p.tx.version,"locktime":p.tx.locktime,"vin":raw_inputs,"vout":p.tx.outputs.iter().map(|o| json!({"value_sats":o.amount,"scriptpubkey":hex(o.script)})).collect::<Vec<_>>()});
    let features = extract(&raw, "psbt_supplied", Some(subject), &signatures);
    let total_out: u128 =
        p.tx.outputs
            .iter()
            .map(|o| u128::from(o.amount) * 1000)
            .sum();
    if known && total_in > u128::from(MAX_MONEY) * 1000 {
        return Err(invalid("total_input_amount_out_of_range"));
    }
    if known && total_in < total_out {
        return Err(invalid("negative_fee"));
    }
    let fee = known.then(|| total_in - total_out);
    let vsize = if !inputs.is_empty() && all_final {
        let mut base = stripped(&p.tx).len();
        for (i, s) in p.tx.inputs.iter().zip(&final_scripts) {
            let mut length = Vec::new();
            variable(s, &mut length);
            base = base - i.script.len() - 1 + length.len();
        }
        let mut extra = Vec::new();
        if final_witnesses.iter().any(|v| !v.is_empty()) {
            extra.extend_from_slice(&[0, 1]);
            for w in &final_witnesses {
                compact(w.len() as u64, &mut extra);
                for item in w {
                    variable(item, &mut extra);
                }
            }
        }
        Some((base * 4 + extra.len()).div_ceil(4))
    } else {
        None
    };
    let rate = fee.zip(vsize.filter(|v| *v != 0)).map(|(f, v)| {
        let context = Context {
            prec: 32,
            rounding: Rounding::HalfEven,
        };
        Decimal::from(f)
            .div(&Decimal::from(1000), &context)
            .unwrap()
            .div(&Decimal::from(v as u64), &context)
            .unwrap()
            .to_string()
    });
    Ok(
        json!({"schema_version":1,"network":network,"chain":"bitcoin","psbt_version":p.version,"subject_id":subject,"content_sha256":sha256::Hash::hash(data).to_string(),
        "transaction_facts":{"inputs":inputs,"outputs":outputs,"fee_msat":fee.map(|v| v.to_string()),"complete":!inputs.is_empty() && !outputs.is_empty() && known,"collaboration":null,"amount_provenance":"psbt_supplied","amounts_chain_verified":false},
        "features":features,"findings":evaluate(&features,None),
        "totals":{"input_msat":known.then(|| total_in.to_string()),"output_msat":total_out.to_string(),"fee_msat":fee.map(|v| v.to_string()),"final_vsize":vsize,"final_fee_rate_sat_vb":rate},
        "coverage":{"known_input_amounts":inputs.iter().filter(|r| !r["amount_msat"].is_null()).count(),"input_count":inputs.len(),"previous_transaction_hashes_verified":inputs.iter().filter(|r| r["utxo_evidence"] == "previous_transaction_hash_verified").count(),"missing_input_indices":inputs.iter().filter(|r| r["amount_msat"].is_null()).map(|r| r["input_index"].clone()).collect::<Vec<_>>()},
        "validation":{"status":"structurally_valid","network_source":"caller_selected","network_verified":false,"chain_membership_verified":false,"unspentness_verified":false,"signatures_verified":false,"input_metadata":metadata,"output_derivation_metadata_present":p.outputs.iter().any(|r| r.iter().any(|(k,_)| [2,7].contains(&k[0]))),"global_xpubs_present":p.globals.iter().any(|(k,_)| k[0] == 1),"transaction_modifiable_flags":number(&p.globals,6,1,false)?.unwrap_or(0),"validated_fields":["map_framing","transaction_structure","prevout_consistency","provided_redeem_and_witness_scripts","partial_signature_encodings"],"discarded_metadata_is_not_authenticated":true,"limitations":["supplied_utxo_amounts_are_not_chain_verified","network_not_encoded_in_script_pubkeys","no_signing_or_script_execution","no_broadcastability_claim","unsigned_size_not_a_final_fee_rate"]}}),
    )
}

#[cfg(test)]
mod tests;
