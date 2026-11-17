//! lib/addressList.ts: parse a pasted/loaded blob of Bitcoin addresses into a clean,
//! de-duplicated mainnet list for an "address" wallet, and purge key material.
//!
//! Kassiber is watch-only: anything that looks like a private key (WIF, BIP38, xprv-family),
//! an extended public key or a raw public key is counted and dropped — never added to the
//! submitted set and never echoed back in a sample.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Private,
    Public,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Every non-empty, non-key token found, in order, including duplicates.
    pub entries: Vec<String>,
    /// Unique mainnet-looking addresses, order-preserving — the set to submit.
    pub valid: Vec<String>,
    /// Unique tokens that do not look like mainnet Bitcoin addresses.
    pub invalid: Vec<String>,
    /// Count of duplicate valid addresses that were collapsed.
    pub duplicates: usize,
    /// Private keys seen — counted, never accepted.
    pub private_keys: usize,
    /// Extended or raw public keys seen — counted, never accepted.
    pub public_keys: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stripped {
    pub text: String,
    pub private_keys: usize,
    pub public_keys: usize,
}

const BASE58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BECH32: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn is_b58(c: char) -> bool {
    c.is_ascii() && BASE58.contains(&(c as u8))
}

fn all_b58(s: &str, min: usize, max: usize) -> bool {
    let n = s.chars().count();
    (min..=max).contains(&n) && s.chars().all(is_b58)
}

fn is_hex(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_hexdigit())
}

const EXT_PRIV: [&str; 10] = [
    "xprv", "yprv", "zprv", "tprv", "uprv", "vprv", "Yprv", "Zprv", "Uprv", "Vprv",
];
const EXT_PUB: [&str; 10] = [
    "xpub", "ypub", "zpub", "tpub", "upub", "vpub", "Ypub", "Zpub", "Upub", "Vpub",
];

/// `classifyKeyMaterial`: None for anything that is not an obvious key.
pub fn classify_key_material(value: &str) -> Option<KeyKind> {
    let t = value.trim();
    let first = t.chars().next();
    // WIF: `^[59KLc][base58]{50,51}$`.
    let wif = matches!(first, Some('5' | '9' | 'K' | 'L' | 'c')) && all_b58(&t[1..], 50, 51);
    // BIP38: `^6P[base58]{56}$`.
    let bip38 = t.starts_with("6P") && all_b58(&t[2..], 56, 56);
    let ext = |prefixes: &[&str]| {
        prefixes
            .iter()
            .any(|p| t.starts_with(p) && all_b58(&t[4..], 70, 120))
    };
    if wif || bip38 || ext(&EXT_PRIV) {
        return Some(KeyKind::Private);
    }
    let hex_pub = t.is_ascii()
        && ((t.len() == 66 && (t.starts_with("02") || t.starts_with("03")))
            || (t.len() == 130 && t.starts_with("04")))
        && is_hex(t);
    if ext(&EXT_PUB) || hex_pub {
        return Some(KeyKind::Public);
    }
    None
}

// ---- SHA-256 (the web carries its own; no crate dependency here either) ----------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut msg = bytes.to_vec();
    let bit_len = (bytes.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut w = [0u32; 64];
    for chunk in msg.chunks(64) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// The base58check payload (version + body, checksum removed) when the checksum holds.
fn base58check_payload(value: &str) -> Option<Vec<u8>> {
    let mut num: Vec<u8> = Vec::new(); // big-endian magnitude
    for c in value.chars() {
        let digit = BASE58.iter().position(|&b| c.is_ascii() && b == c as u8)? as u32;
        let mut carry = digit;
        for byte in num.iter_mut().rev() {
            let v = (*byte as u32) * 58 + carry;
            *byte = (v & 0xff) as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            num.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    while num.first() == Some(&0) {
        num.remove(0);
    }
    let zeros = value.chars().take_while(|&c| c == '1').count();
    let mut payload = vec![0u8; zeros];
    payload.extend(num);
    if payload.len() < 5 {
        return None;
    }
    let (body, checksum) = payload.split_at(payload.len() - 4);
    let expected = sha256(&sha256(body));
    (checksum == &expected[..4]).then(|| body.to_vec())
}

fn bech32_polymod(values: &[u32]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk: u32 = 1;
    for &v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ffffff) << 5) ^ v;
        for (i, g) in GEN.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

#[derive(PartialEq)]
enum Spec {
    Bech32,
    Bech32m,
}

fn bech32_decode(value: &str) -> Option<(String, Vec<u32>, Spec)> {
    if value != value.to_lowercase() && value != value.to_uppercase() {
        return None;
    }
    let s = value.to_lowercase();
    let sep = s.rfind('1')?;
    if sep < 1 || sep + 7 > s.len() {
        return None;
    }
    let hrp = &s[..sep];
    let mut data = Vec::new();
    for c in s[sep + 1..].chars() {
        data.push(BECH32.iter().position(|&b| c.is_ascii() && b == c as u8)? as u32);
    }
    let mut values: Vec<u32> = hrp.bytes().map(|b| (b as u32) >> 5).collect();
    values.push(0);
    values.extend(hrp.bytes().map(|b| (b as u32) & 31));
    values.extend(&data);
    let spec = match bech32_polymod(&values) {
        1 => Spec::Bech32,
        0x2bc830a3 => Spec::Bech32m,
        _ => return None,
    };
    data.truncate(data.len() - 6);
    Some((hrp.to_owned(), data, spec))
}

fn convert_bits(data: &[u32], from: u32, to: u32, pad: bool) -> Option<Vec<u32>> {
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut out = Vec::new();
    let maxv = (1u32 << to) - 1;
    let max_acc = (1u32 << (from + to - 1)) - 1;
    for &v in data {
        if v >> from != 0 {
            return None;
        }
        acc = ((acc << from) | v) & max_acc;
        bits += from;
        while bits >= to {
            bits -= to;
            out.push((acc >> bits) & maxv);
        }
    }
    if pad {
        if bits > 0 {
            out.push((acc << (to - bits)) & maxv);
        }
    } else if bits >= from || ((acc << (to - bits)) & maxv) != 0 {
        return None;
    }
    Some(out)
}

/// `looksLikeMainnetAddress`: the three mainnet shapes, checksum/witness validated.
pub fn looks_like_mainnet_address(value: &str) -> bool {
    let t = value.trim();
    if t.is_empty() {
        return false;
    }
    if let Some(p) = base58check_payload(t) {
        return p.len() == 21 && (p[0] == 0x00 || p[0] == 0x05);
    }
    let Some((hrp, data, spec)) = bech32_decode(t) else {
        return false;
    };
    if hrp != "bc" || data.is_empty() {
        return false;
    }
    let version = data[0];
    if version > 16 {
        return false;
    }
    let Some(program) = convert_bits(&data[1..], 5, 8, false) else {
        return false;
    };
    if program.len() < 2 || program.len() > 40 {
        return false;
    }
    if version == 0 {
        return spec == Spec::Bech32 && (program.len() == 20 || program.len() == 32);
    }
    spec == Spec::Bech32m
}

fn tokens(input: &str) -> impl Iterator<Item = &str> {
    input
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// `parseAddressList`: split on whitespace, commas and semicolons so paste and CSV both work.
pub fn parse(input: &str) -> Parsed {
    let mut p = Parsed::default();
    for token in tokens(input) {
        match classify_key_material(token) {
            Some(KeyKind::Private) => {
                p.private_keys += 1;
                continue;
            }
            Some(KeyKind::Public) => {
                p.public_keys += 1;
                continue;
            }
            None => {}
        }
        p.entries.push(token.to_owned());
        if looks_like_mainnet_address(token) {
            if p.valid.iter().any(|v| v == token) {
                p.duplicates += 1;
            } else {
                p.valid.push(token.to_owned());
            }
        } else if !p.invalid.iter().any(|v| v == token) {
            p.invalid.push(token.to_owned());
        }
    }
    p
}

/// `stripKeyMaterial`: the surviving tokens one per line, plus how many keys were removed.
pub fn strip_key_material(input: &str) -> Stripped {
    let mut s = Stripped::default();
    let mut survivors: Vec<&str> = Vec::new();
    for token in tokens(input) {
        match classify_key_material(token) {
            Some(KeyKind::Private) => s.private_keys += 1,
            Some(KeyKind::Public) => s.public_keys += 1,
            None => survivors.push(token),
        }
    }
    s.text = survivors.join("\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const P2PKH: &str = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa";
    const P2SH: &str = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy";
    const BECH32_ADDR: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
    const BAD_BASE58: &str = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb";
    const BAD_BECH32: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080";
    const WIF: &str = "5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ";
    const TESTNET_WIF: &str = "cMahea7zqjxrtgAbB7LSGbcQUr1uX1ojuat9jZodMN87JcbXMTcA";
    const XPRV: &str = "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
    const XPUB: &str = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet";
    const HEX_PUBKEY: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    #[test]
    fn accepts_mainnet_shapes() {
        assert!(looks_like_mainnet_address(P2PKH));
        assert!(looks_like_mainnet_address(P2SH));
        assert!(looks_like_mainnet_address(BECH32_ADDR));
        assert!(looks_like_mainnet_address(&BECH32_ADDR.to_uppercase()));
    }

    #[test]
    fn rejects_testnet_bad_checksums_junk_and_keys() {
        assert!(!looks_like_mainnet_address(
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        ));
        assert!(!looks_like_mainnet_address(BAD_BASE58));
        assert!(!looks_like_mainnet_address(BAD_BECH32));
        assert!(!looks_like_mainnet_address("not-an-address"));
        assert!(!looks_like_mainnet_address(""));
        assert!(!looks_like_mainnet_address("Bc1QW508D6"));
        assert!(!looks_like_mainnet_address(WIF));
        assert!(!looks_like_mainnet_address(XPUB));
    }

    #[test]
    fn classifies_keys() {
        assert_eq!(classify_key_material(WIF), Some(KeyKind::Private));
        assert_eq!(classify_key_material(TESTNET_WIF), Some(KeyKind::Private));
        assert_eq!(classify_key_material(XPRV), Some(KeyKind::Private));
        assert_eq!(classify_key_material(XPUB), Some(KeyKind::Public));
        assert_eq!(classify_key_material(HEX_PUBKEY), Some(KeyKind::Public));
        assert_eq!(classify_key_material(P2PKH), None);
        assert_eq!(classify_key_material(P2SH), None);
        assert_eq!(classify_key_material(BECH32_ADDR), None);
    }

    #[test]
    fn splits_on_separators() {
        let r = parse(&format!("{P2PKH}\n{P2SH}, {BECH32_ADDR};"));
        assert_eq!(r.valid, [P2PKH, P2SH, BECH32_ADDR]);
        assert!(r.invalid.is_empty());
        assert_eq!(r.duplicates, 0);
    }

    #[test]
    fn dedupes_and_counts() {
        let r = parse(&format!("{P2PKH}\n{P2PKH}\n{P2SH}\n{P2PKH}"));
        assert_eq!(r.valid, [P2PKH, P2SH]);
        assert_eq!(r.duplicates, 2);
    }

    #[test]
    fn separates_junk() {
        let r = parse(&format!("address,balance\n{P2PKH},0.5\ngarbage"));
        assert_eq!(r.valid, [P2PKH]);
        for j in ["address", "balance", "garbage"] {
            assert!(r.invalid.iter().any(|x| x == j));
        }
    }

    #[test]
    fn keys_never_surface() {
        let r = parse(&format!(
            "{P2PKH}\n{WIF}\n{XPRV}\n{XPUB}\n{HEX_PUBKEY}\n{P2SH}"
        ));
        assert_eq!(r.valid, [P2PKH, P2SH]);
        assert_eq!(r.private_keys, 2);
        assert_eq!(r.public_keys, 2);
        let surfaced: Vec<&String> = r.valid.iter().chain(&r.invalid).chain(&r.entries).collect();
        for k in [WIF, XPRV, XPUB, HEX_PUBKEY] {
            assert!(!surfaced.iter().any(|s| s.as_str() == k));
        }
    }

    #[test]
    fn testnet_wif_not_an_invalid_sample() {
        let r = parse(&format!("{P2PKH}\n{TESTNET_WIF}\nlabel"));
        assert_eq!(r.valid, [P2PKH]);
        assert_eq!(r.private_keys, 1);
        assert_eq!(r.invalid, ["label"]);
        assert!(!r.entries.iter().any(|s| s == TESTNET_WIF));
    }

    #[test]
    fn blank_input() {
        let r = parse("   \n\n  ");
        assert!(r.entries.is_empty() && r.valid.is_empty() && r.invalid.is_empty());
    }

    #[test]
    fn strips_keys() {
        let s = strip_key_material(&format!("{P2PKH}, {WIF}\n{P2SH}\n{XPUB}"));
        assert_eq!(s.text, format!("{P2PKH}\n{P2SH}"));
        assert_eq!((s.private_keys, s.public_keys), (1, 1));
        let s = strip_key_material(&format!("{P2PKH} {P2SH}"));
        assert_eq!(s.text, format!("{P2PKH}\n{P2SH}"));
        assert_eq!((s.private_keys, s.public_keys), (0, 0));
    }

    #[test]
    fn sha256_known_vector() {
        let h = sha256(b"abc");
        assert_eq!(h[..4], [0xba, 0x78, 0x16, 0xbf]);
    }
}
