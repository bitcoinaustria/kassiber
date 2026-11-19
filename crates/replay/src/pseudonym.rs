//! Keyed pseudonym primitives for replay bundles. Which field gets which
//! is decided by the projection schema
//! (docs/reference/replay-bundle.md#what-the-inputs-keep).
//!
//! Generate, don't copy: every output is built from the key and, at most, a
//! piece of public structure that was parsed strictly and is re-emitted from
//! a constant (an allowlisted bech32 HRP, the base58 alphabet character of a
//! checksummed address's first symbol, a `txid:vout` split, the length and
//! case of a hex id). Nothing of the original spelling is copied.
//!
//! Equal inputs give equal pseudonyms within one bundle, so links between
//! rows survive. The key is random per bundle and zeroed after export, so
//! pseudonyms neither reverse nor link across bundles.
use bitcoin_hashes::{hmac, sha256, sha256d, Hash, HashEngine};

const BECH32: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BASE58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Address HRPs of Bitcoin, Liquid and Silent Payments.
const ADDRESS_HRPS: &[&str] = &[
    "bc", "tb", "bcrt", "lq", "tlq", "ex", "tex", "el", "ert", "sp", "tsp", "sprt",
];
/// BOLT 11 invoice HRP prefixes; an amount may follow.
const INVOICE_HRPS: &[&str] = &["lnbcrt", "lntbs", "lnbc", "lntb", "lnsb"];

/// Public chain constants (genesis hashes, Liquid policy assets, the
/// coinbase prevout txid): they select network rules and carry nothing
/// about the book. Emitted from this list, never copied.
pub const PUBLIC_HEX: &[&str] = &[
    "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
    "000000000933ea01ad0ee984209779baaec3ced90fa3f408719526f8d77f4943",
    "00000000da84f2bafbbc53dee25a72ae507ff4914b867c565be350b0da8bf043",
    "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6",
    "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206",
    "1466275836220db2944ca059a3a10ef6fd2ea684b0688d2c379296888a206003",
    "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d",
    "144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49",
    "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225",
    "0000000000000000000000000000000000000000000000000000000000000000",
];

/// Keyed bytes in an `id-`/`label-` pseudonym.
const TAG_BYTES: usize = 16;

pub struct Pseudonymizer {
    key: [u8; 32],
}

/// The key must not outlive the export.
impl Drop for Pseudonymizer {
    fn drop(&mut self) {
        self.key = [0; 32];
        std::hint::black_box(&self.key);
    }
}

impl Pseudonymizer {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// A pseudonymizer with a fresh random key.
    pub fn random() -> Result<Self, String> {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|_| "entropy unavailable".to_owned())?;
        Ok(Self::new(key))
    }

    /// At least `len` keyed bytes for `input` in `domain`.
    fn stream(&self, domain: &str, input: &str, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len + 32);
        let mut counter = 0u32;
        while out.len() < len {
            let mut engine = hmac::HmacEngine::<sha256::Hash>::new(&self.key);
            engine.input(domain.as_bytes());
            engine.input(&[0]);
            engine.input(&counter.to_be_bytes());
            engine.input(input.as_bytes());
            out.extend_from_slice(hmac::Hmac::<sha256::Hash>::from_engine(engine).as_byte_array());
            counter += 1;
        }
        out
    }

    /// `prefix-` and 16 keyed bytes (128 bits): distinct values never share
    /// a pseudonym in practice, so rows keep their identities.
    fn tag(&self, prefix: &str, domain: &str, text: &str) -> String {
        let bytes = self.stream(domain, text, TAG_BYTES);
        format!(
            "{prefix}-{}",
            bytes[..TAG_BYTES]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    }

    /// One pseudonym for a whole authored value; equal values stay equal.
    pub fn label(&self, text: &str) -> String {
        self.tag("label", "text", text)
    }

    /// The one keyed mapping for every identifier, local or outside, so
    /// references between rows stay consistent. Public structure survives
    /// only when parsed strictly: a hex id keeps its length and case, a
    /// `txid:vout` outpoint its split and output index, a checksummed bech32
    /// string its allowlisted HRP, a checksummed base58 string its first
    /// symbol. Anything else becomes `id-…`.
    pub fn id(&self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        let lower = text.to_ascii_lowercase();
        if let Some(public) = PUBLIC_HEX.iter().find(|h| **h == lower) {
            return public.to_string();
        }
        if is_hex(text) {
            return self.hex(text);
        }
        if let Some((txid, written)) = text.rsplit_once(':') {
            // Only a canonical decimal vout survives, printed from its value.
            let vout = written
                .parse::<u32>()
                .ok()
                .filter(|v| v.to_string() == written);
            if let (Some(vout), true) = (vout, txid.len() == 64 && is_hex(txid)) {
                return format!("{}:{vout}", self.id(txid));
            }
        }
        if let Some((hrp, len, upper)) = bech32_shape(text) {
            return self.bech32(hrp, &lower, len, upper);
        }
        if let Some(first) = base58check_first(text) {
            return self.base58(first, text);
        }
        self.tag("id", "id", text)
    }

    fn hex(&self, text: &str) -> String {
        let bytes = self.stream("hex", &text.to_ascii_lowercase(), text.len());
        let upper = text.bytes().any(|b| b.is_ascii_uppercase());
        bytes
            .iter()
            .take(text.len())
            .map(|b| {
                let c = char::from(b"0123456789abcdef"[usize::from(b & 15)]);
                if upper {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            })
            .collect()
    }

    fn bech32(&self, hrp: &'static str, input: &str, len: usize, upper: bool) -> String {
        let bytes = self.stream("bech32", input, len);
        let data: String = bytes
            .iter()
            .take(len)
            .map(|b| char::from(BECH32[usize::from(b & 31)]))
            .collect();
        let text = format!("{hrp}1{data}");
        if upper {
            text.to_ascii_uppercase()
        } else {
            text
        }
    }

    fn base58(&self, first: u8, text: &str) -> String {
        let bytes = self.stream("base58", text, text.len());
        std::iter::once(char::from(first))
            .chain(
                bytes
                    .iter()
                    .skip(1)
                    .take(text.len() - 1)
                    .map(|b| char::from(BASE58[usize::from(*b) % BASE58.len()])),
            )
            .collect()
    }
}

fn is_hex(text: &str) -> bool {
    text.len() >= 16 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn polymod(values: &[u8]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk: u32 = 1;
    for v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ffffff) << 5) ^ u32::from(*v);
        for (i, g) in GEN.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

/// A checksummed bech32 or bech32m string whose HRP is allowlisted: the
/// allowlisted HRP constant, the data length and the case.
fn bech32_shape(text: &str) -> Option<(&'static str, usize, bool)> {
    let lower = text.to_ascii_lowercase();
    if text != lower && text != text.to_ascii_uppercase() {
        return None;
    }
    let at = lower.rfind('1')?;
    let (hrp, data) = (&lower[..at], &lower[at + 1..]);
    if data.len() < 6 || hrp.is_empty() {
        return None;
    }
    let values: Vec<u8> = data
        .bytes()
        .map(|b| BECH32.iter().position(|c| *c == b).map(|p| p as u8))
        .collect::<Option<_>>()?;
    let mut expanded: Vec<u8> = hrp.bytes().map(|b| b >> 5).collect();
    expanded.push(0);
    expanded.extend(hrp.bytes().map(|b| b & 31));
    expanded.extend(&values);
    let check = polymod(&expanded);
    if check != 1 && check != 0x2bc830a3 {
        return None;
    }
    let constant = ADDRESS_HRPS
        .iter()
        .find(|h| **h == hrp)
        .copied()
        .or_else(|| {
            INVOICE_HRPS
                .iter()
                .find(|prefix| {
                    hrp.strip_prefix(**prefix).is_some_and(|amount| {
                        let digits = amount.trim_end_matches(['m', 'u', 'n', 'p']);
                        amount.len() - digits.len() <= 1
                            && digits.bytes().all(|b| b.is_ascii_digit())
                    })
                })
                .copied()
        })?;
    Some((constant, data.len(), text != lower))
}

/// The first symbol of a base58check string, as an alphabet constant.
fn base58check_first(text: &str) -> Option<u8> {
    if text.len() < 25 || text.len() > 120 {
        return None;
    }
    let mut bytes: Vec<u8> = Vec::new();
    for c in text.bytes() {
        let mut carry = BASE58.iter().position(|a| *a == c)? as u32;
        for b in bytes.iter_mut().rev() {
            carry += u32::from(*b) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let zeros = text.bytes().take_while(|b| *b == b'1').count();
    let mut decoded = vec![0u8; zeros];
    decoded.extend(bytes);
    if decoded.len() < 5 {
        return None;
    }
    let (payload, checksum) = decoded.split_at(decoded.len() - 4);
    let hash = sha256d::Hash::hash(payload);
    if &hash.as_byte_array()[..4] != checksum {
        return None;
    }
    let first = text.as_bytes()[0];
    BASE58.iter().copied().find(|c| *c == first)
}

/// SHA-256 of `bytes`, lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    sha256::Hash::hash(bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TXID: &str = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";

    fn p() -> Pseudonymizer {
        Pseudonymizer::new([7; 32])
    }

    #[test]
    fn only_strictly_parsed_public_structure_survives() {
        let p = p();
        let a = p.id(TXID);
        assert_eq!(a.len(), 64);
        assert_ne!(a, TXID);
        assert_eq!(p.id(&format!("{TXID}:3")), format!("{a}:3"));
        assert_eq!(p.id(&TXID.to_uppercase()), a.to_uppercase());
        // A valid address keeps its HRP; a valid base58check its first symbol.
        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
        let pseudo = p.id(address);
        assert!(
            pseudo.starts_with("bc1q") || pseudo.starts_with("bc1"),
            "{pseudo}"
        );
        assert_ne!(pseudo, address);
        let legacy = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2";
        let pseudo = p.id(legacy);
        assert!(pseudo.starts_with('1') && pseudo.len() == legacy.len() && pseudo != legacy);
        // Look-alikes without a valid checksum or an allowlisted HRP, uniform
        // hex, UUIDs, mixed ids and local ids are replaced whole.
        for value in [
            "lnalicesentinelqqqqqqqqqqqqqqqqqqqqqq".to_owned(),
            "bc1qalicesentinel000000".to_owned(),
            "a".repeat(64),
            "6f9d6b8a-5b7e-4c1a-9d0e-1f2a3b4c5d6e".to_owned(),
            format!("SENTINEL-ALICE-{TXID}"),
            "alice.private.example".to_owned(),
            "x-in".to_owned(),
            format!("{TXID}:03"),
        ] {
            let pseudo = p.id(&value);
            assert!(
                pseudo.starts_with("id-")
                    || (pseudo.len() == value.len() && !pseudo.contains("alice")),
                "{value} -> {pseudo}"
            );
            assert!(
                !pseudo.contains("alice") && !pseudo.starts_with("ln") && pseudo != value,
                "{value} -> {pseudo}"
            );
            assert_eq!(pseudo, p.id(&value));
        }
        assert_eq!(p.id(&"0".repeat(64)), "0".repeat(64));
        assert!(p.label("Alice").starts_with("label-"));
    }

    /// Whole-value pseudonyms keep 128 keyed bits: two ids that shared a
    /// 40-bit tag under this key stay distinct.
    #[test]
    fn whole_value_pseudonyms_do_not_collide() {
        let p = Pseudonymizer::new([7; 32]);
        let (a, b) = (p.id("transaction-1483887"), p.id("transaction-1897022"));
        assert_ne!(a, b);
        assert_eq!(a.len(), "id-".len() + 2 * TAG_BYTES);
        assert_eq!(p.label("x").len(), "label-".len() + 2 * TAG_BYTES);
    }

    #[test]
    fn different_keys_give_unrelated_pseudonyms() {
        assert_ne!(
            Pseudonymizer::new([1; 32]).id(TXID),
            Pseudonymizer::new([2; 32]).id(TXID)
        );
    }
}
