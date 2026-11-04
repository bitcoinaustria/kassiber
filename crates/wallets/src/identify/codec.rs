//! The small encodings identification reads, each matching its Python helper:
//! `core.address_scripts`, BOLT11 payment hashes, BIP352 addresses, and the
//! `bytes.fromhex`/`str.splitlines` rules token parsing depends on.
use sha2::{Digest, Sha256};

const CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32M: u64 = 0x2bc8_30a3;

fn polymod(values: &[u64]) -> u64 {
    const GENERATOR: [u64; 5] = [
        0x3b6a_57b2,
        0x2650_8e6d,
        0x1ea1_19fa,
        0x3d42_33dd,
        0x2a14_62b3,
    ];
    let mut checksum = 1u64;
    for value in values {
        let top = checksum >> 25;
        checksum = ((checksum & 0x1ff_ffff) << 5) ^ value;
        for (bit, generator) in GENERATOR.iter().enumerate() {
            if (top >> bit) & 1 == 1 {
                checksum ^= generator;
            }
        }
    }
    checksum
}

fn hrp_expand(hrp: &str) -> Vec<u64> {
    let mut out: Vec<u64> = hrp.chars().map(|c| u64::from(c as u32) >> 5).collect();
    out.push(0);
    out.extend(hrp.chars().map(|c| u64::from(c as u32) & 31));
    out
}

fn words(data: &str) -> Option<Vec<u64>> {
    data.chars()
        .map(|c| CHARSET.find(c).map(|i| i as u64))
        .collect()
}

/// BIP173's `convertbits`; `None` where Python raises or returns `None`.
fn convert_bits(data: &[u64], from: u32, to: u32, pad: bool) -> Option<Vec<u8>> {
    let (mut acc, mut bits) = (0u64, 0u32);
    let maximum = (1u64 << to) - 1;
    let maximum_acc = (1u64 << (from + to - 1)) - 1;
    let mut out = vec![];
    for value in data {
        if value >> from != 0 {
            return None;
        }
        acc = ((acc << from) | value) & maximum_acc;
        bits += from;
        while bits >= to {
            bits -= to;
            out.push(((acc >> bits) & maximum) as u8);
        }
    }
    if pad {
        if bits > 0 {
            out.push(((acc << (to - bits)) & maximum) as u8);
        }
    } else if bits >= from || (acc << (to - bits)) & maximum != 0 {
        return None;
    }
    Some(out)
}

fn mixed_case(text: &str) -> bool {
    text != text.to_lowercase() && text != text.to_uppercase()
}

fn sha256d(payload: &[u8]) -> [u8; 32] {
    Sha256::digest(Sha256::digest(payload)).into()
}

fn base58check(value: &str) -> Option<Vec<u8>> {
    const ALPHABET: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    // Big-endian base-256 accumulator, like Python's unbounded integer.
    let mut number: Vec<u8> = vec![];
    for char in value.chars() {
        let mut carry = ALPHABET.find(char)? as u32;
        for byte in number.iter_mut().rev() {
            carry += u32::from(*byte) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            number.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let leading = value.chars().take_while(|c| *c == '1').count();
    let mut payload = vec![0u8; leading];
    payload.extend(number.into_iter().skip_while(|b| *b == 0));
    if payload.len() < 5 {
        return None;
    }
    let (body, checksum) = payload.split_at(payload.len() - 4);
    (sha256d(body)[..4] == *checksum).then(|| body.to_vec())
}

/// `address_scripts.address_to_scriptpubkey(address).hex()`, `None` where it
/// raises. Bitcoin only; Liquid addresses never canonicalize here.
pub fn address_script(address: &str) -> Option<String> {
    let lower = address.to_lowercase();
    let script = if ["bc1", "tb1", "bcrt1"].iter().any(|p| lower.starts_with(p)) {
        if mixed_case(address) {
            return None;
        }
        let separator = lower.rfind('1')?;
        let length = lower.chars().count();
        let separator_chars = lower[..separator].chars().count();
        if separator_chars < 1 || separator_chars + 7 > length {
            return None;
        }
        let hrp = &lower[..separator];
        let data = words(&lower[separator + 1..])?;
        let mut checked = hrp_expand(hrp);
        checked.extend(&data);
        let bech32m = match polymod(&checked) {
            1 => false,
            BECH32M => true,
            _ => return None,
        };
        let data = &data[..data.len() - 6];
        let (&version, program) = data.split_first()?;
        if version > 16 {
            return None;
        }
        let program = convert_bits(program, 5, 8, false)?;
        if program.len() < 2 || program.len() > 40 {
            return None;
        }
        if version == 0 && (![20, 32].contains(&program.len()) || bech32m) {
            return None;
        }
        if version > 0 && !bech32m {
            return None;
        }
        let opcode = if version == 0 {
            0
        } else {
            0x50 + version as u8
        };
        let mut script = vec![opcode, program.len() as u8];
        script.extend(program);
        script
    } else {
        let payload = base58check(address)?;
        let (&version, hash) = payload.split_first()?;
        if hash.len() != 20 {
            return None;
        }
        match version {
            0x00 | 0x6f => [&[0x76, 0xa9, 0x14][..], hash, &[0x88, 0xac]].concat(),
            0x05 | 0xc4 => [&[0xa9, 0x14][..], hash, &[0x87]].concat(),
            _ => return None,
        }
    };
    Some(script.iter().map(|b| format!("{b:02x}")).collect())
}

/// `bytes.fromhex(text)` succeeds: ASCII whitespace may separate byte pairs.
pub fn is_hex(text: &str) -> bool {
    let mut bytes = text.bytes().peekable();
    loop {
        while bytes.next_if(|b| b" \t\n\r\x0b\x0c".contains(b)).is_some() {}
        let Some(high) = bytes.next() else {
            return true;
        };
        if !high.is_ascii_hexdigit() || !bytes.next().is_some_and(|b| b.is_ascii_hexdigit()) {
            return false;
        }
    }
}

/// `str.splitlines()`.
pub fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = vec![];
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, char)) = chars.next() {
        if matches!(
            char,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            lines.push(&text[start..at]);
            let mut end = at + char.len_utf8();
            if char == '\r' && chars.next_if(|(_, next)| *next == '\n').is_some() {
                end += 1;
            }
            start = end;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// What a BOLT11 invoice names; the invoice itself is not kept.
pub struct Invoice {
    pub payment_hash: String,
    pub network: &'static str,
}

fn without_scheme(text: &str) -> &str {
    let head: String = text.chars().take(10).collect();
    if head.to_lowercase() == "lightning:" {
        &text[head.len()..]
    } else {
        text
    }
}

/// `lightning.bolt11.decode_payment_hash`. Signatures are not verified.
pub fn decode_invoice(invoice: &str) -> Option<Invoice> {
    let text = without_scheme(kassiber_pytext::strip(invoice));
    if mixed_case(text) {
        return None;
    }
    let text = text.to_lowercase();
    let separator = text
        .rfind('1')
        .filter(|at| text[..*at].chars().count() >= 3)?;
    let (hrp, data) = (&text[..separator], &text[separator + 1..]);
    let currency = hrp.strip_prefix("ln")?;
    let network = [
        ("bcrt", "regtest"),
        ("bc", "main"),
        ("tbs", "signet"),
        ("tb", "test"),
        ("sb", "simnet"),
    ]
    .into_iter()
    .find_map(|(prefix, network)| {
        let amount = currency.strip_prefix(prefix)?;
        let digits = amount.strip_suffix(['m', 'u', 'n', 'p']).unwrap_or(amount);
        Some((digits.chars().all(char::is_numeric), network))
    })
    .and_then(|(valid, network)| valid.then_some(network))?;
    let values = words(data)?;
    if values.len() < 7 + 104 + 6 {
        return None;
    }
    let mut checked = hrp_expand(hrp);
    checked.extend(&values);
    if polymod(&checked) != 1 {
        return None;
    }
    let fields = &values[..values.len() - 110];
    let mut index = 7;
    let mut payment_hash = None;
    while index + 3 <= fields.len() {
        let tag = fields[index];
        let length = (fields[index + 1] * 32 + fields[index + 2]) as usize;
        let field = &fields[(index + 3).min(fields.len())..(index + 3 + length).min(fields.len())];
        index += 3 + length;
        if index > fields.len() {
            return None;
        }
        if tag == 1 && length == 52 {
            let bytes = convert_bits(field, 5, 8, true)?;
            // Python's `_words_to_bytes` drops the partial trailing byte.
            let whole = field.len() * 5 / 8;
            payment_hash = Some(
                bytes[..whole.min(32)]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            );
        }
    }
    Some(Invoice {
        payment_hash: payment_hash?,
        network,
    })
}

/// `bolt11.display_invoice`: a short stand-in, never the reusable invoice.
pub fn display_invoice(invoice: &str) -> String {
    let text = without_scheme(kassiber_pytext::strip(invoice));
    let chars: Vec<char> = text.chars().collect();
    if chars.len() > 20 {
        format!(
            "{}…{}",
            chars[..12].iter().collect::<String>(),
            chars[chars.len() - 6..].iter().collect::<String>()
        )
    } else {
        text.to_owned()
    }
}

/// `silent_payments._bech32m_payload`: version-0 bech32m without BIP173's
/// length cap.
pub fn bech32m_payload(text: &str, hrps: &[&str]) -> Option<(String, Vec<u8>)> {
    let value = kassiber_pytext::strip(text);
    if mixed_case(value) {
        return None;
    }
    let value = value.to_lowercase();
    let separator = value.rfind('1')?;
    let (hrp, data) = (&value[..separator], &value[separator + 1..]);
    if separator < 1 || !hrps.contains(&hrp) || data.chars().count() < 7 {
        return None;
    }
    let values = words(data)?;
    let mut checked = hrp_expand(hrp);
    checked.extend(&values);
    if polymod(&checked) != BECH32M || values[0] != 0 {
        return None;
    }
    let payload = convert_bits(&values[1..values.len() - 6], 5, 8, false)?;
    Some((hrp.to_owned(), payload))
}

/// `silent_payments.decode_address`: (network family, B_scan, B_spend).
pub fn decode_silent_payment_address(text: &str) -> Option<(&'static str, Vec<u8>, Vec<u8>)> {
    let (hrp, payload) = bech32m_payload(text, &["sp", "tsp", "sprt"])?;
    if payload.len() != 66 {
        return None;
    }
    let family = match hrp.as_str() {
        "sp" => "main",
        "tsp" => "test",
        _ => "regtest",
    };
    Some((family, payload[..33].to_vec(), payload[33..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_scripts_follow_the_python_helper() {
        assert_eq!(
            address_script("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq").as_deref(),
            Some("0014e8df018c7e326cc253faac7e46cdc51e68542c42")
        );
        assert_eq!(
            address_script("BC1QAR0SRRR7XFKVY5L643LYDNW9RE59GTZZWF5MDQ").as_deref(),
            Some("0014e8df018c7e326cc253faac7e46cdc51e68542c42")
        );
        assert_eq!(
            address_script("bc1qAr0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"),
            None
        );
        assert_eq!(
            address_script("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2").as_deref(),
            Some("76a91477bff20c60e522dfaa3350c39b030a5d004e839a88ac")
        );
        assert_eq!(
            address_script("3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy").as_deref(),
            Some("a914b472a266d0bd89c13706a4132ccfb16f7c3b9fcb87")
        );
        assert_eq!(address_script("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN3"), None);
        assert_eq!(
            address_script("bc1pdqxk5pjfm2c5ellt67u9rjpajde2qlpjsgc244u6p4qpatf2ug6sv3hmk7")
                .as_deref(),
            Some("5120680d6a0649dab14cffebd7b851c83d9372a07c328230aad79a0d401ead2ae235")
        );
        assert_eq!(address_script("lq1qqbogus"), None);
        assert_eq!(address_script(""), None);
    }

    #[test]
    fn fromhex_and_splitlines_follow_python() {
        assert!(is_hex("00aa"));
        assert!(is_hex(" 00 aa "));
        assert!(is_hex(""));
        assert!(!is_hex("0 0"));
        assert!(!is_hex("0g"));
        assert!(!is_hex("abc"));
        assert_eq!(
            split_lines("a\r\nb\rc\nd\u{2028}e\x0bf\n"),
            ["a", "b", "c", "d", "e", "f"]
        );
        assert_eq!(split_lines("\n\nx"), ["", "", "x"]);
        assert!(split_lines("").is_empty());
    }

    #[test]
    fn invoices_keep_only_their_payment_hash() {
        // BOLT11's "please make a donation of any amount" example.
        let invoice = "lnbc1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdpl2pkx2ctnv5sxxmmwwd5kgetjypeh2ursdae8g6twvus8g6rfwvs8qun0dfjkxaq9qrsgq357wnc5r2ueh7ck6q93dj32dlqnls087fxdwk8qakdyafkq3yap9us6v52vjjsrvywa6rt52cm9r9zqt8r2t7mlcwspyetp5h2tztugp9lfyql";
        let decoded = decode_invoice(invoice).unwrap();
        assert_eq!(decoded.network, "main");
        assert_eq!(
            decoded.payment_hash,
            "0001020304050607080900010203040506070809000102030405060708090102"
        );
        assert_eq!(
            decode_invoice(&invoice.to_uppercase()).unwrap().network,
            "main"
        );
        assert!(decode_invoice(&format!("lightning:{invoice}")).is_some());
        assert!(decode_invoice(&invoice.replace("lnbc1p", "lnbc1q")).is_none());
        assert_eq!(display_invoice(invoice), "lnbc1pvjluez…9lfyql");
        assert_eq!(display_invoice("lightning:short"), "short");
    }
}
