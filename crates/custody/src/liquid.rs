//! Compatibility with `onchain._decode_liquid_raw_hex` and embit 0.8.0.
//!
//! This is a public-structure projection, never consensus validation or
//! unblinding. Elements' consensus decoder rejects several inputs the Python
//! reference admits (flags, varints, commitments, short locktime/proofs). A
//! bounded cursor over the existing bytes preserves that boundary without a
//! crypto dependency or allocations driven by untrusted length prefixes.
use crate::python_text::PythonText;
use serde_json::{json, Value};

struct Cursor<'a>(&'a [u8]);
impl<'a> Cursor<'a> {
    // BytesIO.read permits short reads, but rejects lengths above Py_ssize_t.
    fn read(&mut self, count: u64) -> Option<&'a [u8]> {
        if count > i64::MAX as u64 {
            return None;
        }
        let count = usize::try_from(count).ok()?.min(self.0.len());
        let (head, tail) = self.0.split_at(count);
        self.0 = tail;
        Some(head)
    }
    fn byte(&mut self) -> Option<u8> {
        self.read(1)?.first().copied()
    }
    fn compact(&mut self) -> Option<u64> {
        let first = self.byte()?;
        // embit neither checks minimal encodings nor requires all payload bytes.
        if first < 0xfd {
            Some(u64::from(first))
        } else {
            Some(little(self.read(1 << (first - 0xfc))?))
        }
    }
    fn script(&mut self) -> Option<&'a [u8]> {
        let count = self.compact()?;
        let data = self.read(count)?;
        (data.len() as u64 == count).then_some(data)
    }
    fn proof(&mut self) -> Option<()> {
        let count = self.compact()?;
        self.read(count)?; // Proof and Witness items deliberately allow short data.
        Some(())
    }
    fn witness(&mut self) -> Option<()> {
        for _ in 0..self.compact()? {
            self.proof()?;
        }
        Some(())
    }
    fn issuance_commitment(&mut self) -> Option<()> {
        let count = match self.byte()? {
            0 => 0,
            1 => 8,
            _ => 32,
        };
        (self.read(count)?.len() as u64 == count).then_some(())
    }
}
fn little(data: &[u8]) -> u64 {
    data.iter()
        .enumerate()
        .fold(0, |n, (i, byte)| n | (u64::from(*byte) << (i * 8)))
}
fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(raw: &str) -> Option<Vec<u8>> {
    let raw = raw.py_strip();
    if raw.is_empty() || !raw.len().is_multiple_of(2) {
        return None;
    }
    raw.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |c: u8| match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                b'A'..=b'F' => Some(c - b'A' + 10),
                _ => None,
            };
            Some(digit(pair[0])? * 16 + digit(pair[1])?)
        })
        .collect()
}

pub(crate) fn decode(raw: &Value) -> Option<Value> {
    let bytes = unhex(raw.as_str()?)?;
    let mut cursor = Cursor(&bytes);
    cursor.read(4)?;
    let witness = cursor.byte()? == 1;
    let mut inputs = Vec::new();
    for _ in 0..cursor.compact()? {
        let mut txid = cursor.read(32)?.to_vec();
        txid.reverse();
        let mut vout = little(cursor.read(4)?);
        cursor.script()?;
        cursor.read(4)?;
        if vout != 0xffff_ffff {
            if vout & (1 << 31) != 0 {
                if cursor.read(32)?.len() != 32 || cursor.read(32)?.len() != 32 {
                    return None;
                }
                cursor.issuance_commitment()?;
                cursor.issuance_commitment()?;
            }
            vout &= 0x3fff_ffff;
        }
        inputs.push(
            json!({"outpoint":if txid.is_empty(){None}else{Some(format!("{}:{vout}", hex(&txid)))},
            "script":null,"value_sats":null,"asset_id":null,"asset":null}),
        );
    }
    let mut outputs = Vec::new();
    for n in 0..cursor.compact()? {
        let asset = cursor.read(33)?;
        let value = if cursor.byte()? == 1 {
            let mut data = cursor.read(8)?.to_vec();
            data.reverse();
            Some(little(&data))
        } else {
            cursor.read(32)?;
            None
        };
        let blinded = cursor.byte()? != 0;
        if blinded {
            cursor.read(32)?;
        }
        let script = hex(cursor.script()?);
        let asset_id = if !blinded && asset.len() == 33 && asset[0] == 1 {
            let mut data = asset[1..].to_vec();
            data.reverse();
            Some(hex(&data))
        } else {
            None
        };
        outputs.push(
            json!({"n":n,"script":script,"value_sats":if blinded{None}else{value},
            "asset_id":asset_id,"asset":null,"role":if script.is_empty(){Some("fee")}else{None}}),
        );
    }
    cursor.read(4)?; // Python accepts zero through four locktime bytes.
    if witness {
        for _ in &inputs {
            cursor.proof()?;
            cursor.proof()?;
            cursor.witness()?;
            cursor.witness()?;
        }
        for _ in &outputs {
            cursor.proof()?;
            cursor.proof()?;
        }
    }
    if !cursor.0.is_empty() {
        return None;
    }
    Some(
        json!({"txid":null,"chain":"liquid","network":"","inputs":inputs,
        "outputs":outputs,"component":{},"evidence_conflicts":[]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(group: &str) {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_liquid_cases.json"
        ))
        .unwrap();
        let mut count = 0;
        for case in cases.iter().filter(|case| case["group"] == group) {
            assert_eq!(
                decode(&case["input"]),
                case["expected"]
                    .as_object()
                    .map(|_| case["expected"].clone()),
                "{}",
                case["case"]
            );
            count += 1;
        }
        assert!(count > 0);
    }
    macro_rules! groups {
        ($($name:ident => $group:literal),* $(,)?) => {$(#[test] fn $name(){check($group);})*};
    }
    groups! {
        liquid_valid => "valid",
        liquid_truncation => "truncation",
        liquid_flags => "flags",
        liquid_prefixes => "prefixes",
        liquid_varints => "varints",
        liquid_issuance => "issuance",
        liquid_witness => "witness",
        liquid_hex => "hex",
        liquid_seeded => "seeded",
    }
}
