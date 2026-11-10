use super::*;

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub(super) fn encode(value: &Value, ascii: bool) -> Result<Value> {
    let mut out = String::new();
    let encoded = dumps(
        value,
        &Options::new()
            .sort_keys(true)
            .compact()
            .ensure_ascii(ascii)
            .allow_nan(false),
    )
    .map_err(|_| invalid("Invalid chain analysis cursor"))?;
    for chunk in encoded.as_bytes().chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i > chunk.len() {
                '='
            } else {
                ALPHABET[((bits >> (18 - i * 6)) & 63) as usize] as char
            });
        }
    }
    Ok(s(out))
}

pub(super) fn decode(raw: &str) -> Option<Value> {
    if !raw.is_ascii() {
        return None;
    }
    let mut out = Vec::new();
    let (mut bits, mut count, mut padding) = (0u32, 0, 0);
    for b in raw.bytes() {
        if b == b'=' {
            if count >= 2 {
                padding += 1;
                if count + padding >= 4 {
                    count = 0;
                    break;
                }
            }
            continue;
        }
        let b = match b {
            b'+' => b'-',
            b'/' => b'_',
            b => b,
        };
        let Some(n) = ALPHABET.iter().position(|v| *v == b) else {
            continue;
        };
        padding = 0;
        bits = (bits << 6) | n as u32;
        count += 1;
        if count >= 2 {
            out.push((bits >> (8 - 2 * count)) as u8);
        }
        if count == 4 {
            count = 0;
            bits = 0;
        }
    }
    if count != 0 {
        return None;
    }
    kassiber_pytext::loads(&String::from_utf8(out).ok()?).ok()
}
