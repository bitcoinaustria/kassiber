//! lib/bbqrWalletMaterial.ts: what one scanned QR frame means for the wallet-material scanner.
//! A plain QR is the material itself (Auto / Single QR); a BBQR frame (`B$` + encoding + file
//! type + base-36 total and index) joins a collection that decodes once every part is in (HEX,
//! Base32, or Base32 + raw DEFLATE with a 1 KB window), and only text, JSON or a recognised
//! wallet export is accepted. The messages are the web's, untranslated as there.

use std::collections::BTreeMap;

use super::libs::wallet_material::{Kind, detect};

const BBQR_PREFIX: &str = "B$";
const BBQR_HEADER_LENGTH: usize = 8;
const BASE32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// `QrScanMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanMode {
    #[default]
    Auto,
    Single,
    Bbqr,
}

/// `BbqrCollectorState`: frames by index, and the header every frame must share.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collector {
    pub frames: BTreeMap<u32, String>,
    pub total: Option<u32>,
    pub encoding: Option<char>,
    pub file_type: Option<char>,
}

/// `BbqrProgress`.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub received: u32,
    pub total: u32,
    pub file_type: String,
}

/// `WalletMaterialScanResult`.
#[derive(Clone, Debug, PartialEq)]
pub enum ScanResult {
    Single(String),
    Progress {
        state: Collector,
        progress: Progress,
    },
    Complete {
        state: Collector,
        progress: Progress,
        material: String,
    },
    Ignored(&'static str),
    Error(String),
}

struct Frame {
    total: u32,
    index: u32,
    encoding: char,
    file_type: char,
    payload: String,
}

fn base36_pair(v: &str) -> Option<u32> {
    (v.len() == 2
        && v.bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase()))
    .then(|| u32::from_str_radix(v, 36).ok())
    .flatten()
}

/// `isBbqrFrame`.
#[allow(dead_code)]
pub fn is_bbqr_frame(value: &str) -> bool {
    value.trim().to_uppercase().starts_with(BBQR_PREFIX)
}

fn parse_frame(value: &str) -> Option<Frame> {
    let t = value.trim().to_uppercase();
    if !t.starts_with(BBQR_PREFIX)
        || t.chars().count() < BBQR_HEADER_LENGTH
        || !t.is_char_boundary(BBQR_HEADER_LENGTH)
    {
        return None;
    }
    let total = base36_pair(t.get(4..6)?)?;
    let index = base36_pair(t.get(6..8)?)?;
    if total < 1 || index >= total {
        return None;
    }
    let mut c = t.chars();
    let encoding = c.nth(2)?;
    let file_type = c.next()?;
    Some(Frame {
        total,
        index,
        encoding,
        file_type,
        payload: t[BBQR_HEADER_LENGTH..].to_owned(),
    })
}

fn progress_for(state: &Collector) -> Progress {
    let received = state.frames.len() as u32;
    Progress {
        received,
        total: state.total.unwrap_or(received),
        file_type: state
            .file_type
            .map(String::from)
            .unwrap_or_else(|| "?".into()),
    }
}

fn decode_hex(v: &str) -> Result<Vec<u8>, String> {
    let err = || "Could not decode BBQR HEX payload.".to_owned();
    if !v.len().is_multiple_of(2)
        || !v
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
    {
        return Err(err());
    }
    (0..v.len() / 2)
        .map(|i| u8::from_str_radix(&v[i * 2..i * 2 + 2], 16).map_err(|_| err()))
        .collect()
}

fn decode_base32(v: &str) -> Result<Vec<u8>, String> {
    let err = || "Could not decode BBQR Base32 payload.".to_owned();
    let mut out = Vec::with_capacity(v.len() * 5 / 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for ch in v.bytes() {
        let n = BASE32_ALPHABET
            .iter()
            .position(|&a| a == ch)
            .ok_or_else(err)? as u32;
        buffer = ((buffer << 5) | n) & 0xffff;
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    if bits > 0 && buffer & ((1 << bits) - 1) != 0 {
        return Err("Could not decode BBQR Base32 padding bits.".into());
    }
    Ok(out)
}

/// `decodeBbqrBytes`: the frames joined in index order, then decoded per the first's encoding.
fn decode_bytes(parts: &[&str]) -> Result<(char, Vec<u8>), String> {
    let mut frames = Vec::with_capacity(parts.len());
    for p in parts {
        frames.push(parse_frame(p).ok_or("Could not decode malformed BBQR frames.")?);
    }
    let first = frames
        .first()
        .ok_or("The scanned BBQR payload was empty.")?;
    let (encoding, file_type) = (first.encoding, first.file_type);
    frames.sort_by_key(|f| f.index);
    let payload: String = frames.iter().map(|f| f.payload.as_str()).collect();
    let raw = match encoding {
        'H' => decode_hex(&payload)?,
        '2' => decode_base32(&payload)?,
        'Z' => miniz_oxide::inflate::decompress_to_vec(&decode_base32(&payload)?)
            .map_err(|_| "invalid stored block lengths".to_owned())?,
        other => return Err(format!("BBQR encoding {other} is not supported.")),
    };
    Ok((file_type, raw))
}

/// `decodeBbqrMaterial`: UTF-8 text of type U (text) or J (JSON), or B when it reads as a
/// wallet export or descriptor.
fn decode_material(parts: &[&str]) -> Result<String, String> {
    let (file_type, raw) = decode_bytes(parts)?;
    let text = String::from_utf8(raw)
        .map_err(|_| "The encoded data was not valid for encoding utf-8".to_owned())?;
    let material = text.trim().to_owned();
    if material.is_empty() {
        return Err("The scanned BBQR payload was empty.".into());
    }
    if matches!(file_type, 'U' | 'J') {
        return Ok(material);
    }
    if file_type == 'B' && !matches!(detect(&material).kind, Kind::Unknown | Kind::Empty) {
        return Ok(material);
    }
    Err(format!(
        "BBQR type {file_type} is not a wallet export or descriptor."
    ))
}

/// `processWalletMaterialQrScan`.
pub fn process(value: &str, mode: ScanMode, current: &Collector) -> ScanResult {
    let scanned = value.trim();
    if scanned.is_empty() {
        return ScanResult::Ignored("Empty QR frame ignored.");
    }
    let Some(parsed) = parse_frame(scanned) else {
        return if mode == ScanMode::Bbqr {
            ScanResult::Ignored("Waiting for a BBQR frame.")
        } else {
            ScanResult::Single(scanned.to_owned())
        };
    };
    if mode == ScanMode::Single {
        return ScanResult::Ignored("BBQR frame seen; switch to Auto or BBQR to collect it.");
    }
    let mismatch = current.total.is_some_and(|t| t != parsed.total)
        || current.encoding.is_some_and(|e| e != parsed.encoding)
        || current.file_type.is_some_and(|f| f != parsed.file_type);
    let mut state = if mismatch {
        Collector::default()
    } else {
        current.clone()
    };
    state.frames.insert(parsed.index, scanned.to_owned());
    state.total = Some(parsed.total);
    state.encoding = Some(parsed.encoding);
    state.file_type = Some(parsed.file_type);
    let progress = progress_for(&state);
    if progress.received < parsed.total {
        return ScanResult::Progress { state, progress };
    }
    let parts: Vec<&str> = state.frames.values().map(String::as_str).collect();
    match decode_material(&parts) {
        Ok(material) => ScanResult::Complete {
            state,
            progress,
            material,
        },
        Err(e) => ScanResult::Error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b32(bytes: &[u8]) -> String {
        let mut out = String::new();
        let (mut buffer, mut bits) = (0u32, 0u32);
        for &b in bytes {
            buffer = (buffer << 8) | u32::from(b);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(BASE32_ALPHABET[((buffer >> bits) & 31) as usize] as char);
            }
            buffer &= (1 << bits) - 1;
        }
        if bits > 0 {
            out.push(BASE32_ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
        }
        out
    }

    fn frames(enc: char, ty: char, payload: &str, n: usize) -> Vec<String> {
        let chunk = payload.len().div_ceil(n);
        let chunk = chunk + (8 - chunk % 8) % 8;
        let parts: Vec<&str> = payload
            .as_bytes()
            .chunks(chunk.max(8))
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        let total = parts.len();
        let b36 = |v: usize| format!("{:0>2}", radix36(v));
        parts
            .iter()
            .enumerate()
            .map(|(i, p)| format!("B${enc}{ty}{}{}{p}", b36(total), b36(i)))
            .collect()
    }

    fn radix36(mut v: usize) -> String {
        let digits = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut s = Vec::new();
        loop {
            s.push(digits[v % 36]);
            v /= 36;
            if v == 0 {
                break;
            }
        }
        s.reverse();
        String::from_utf8(s).unwrap()
    }

    fn collect(frames: &[String], mode: ScanMode) -> ScanResult {
        let mut state = Collector::default();
        let mut last = ScanResult::Ignored("none");
        for f in frames {
            last = process(f, mode, &state);
            match &last {
                ScanResult::Progress { state: s, .. } | ScanResult::Complete { state: s, .. } => {
                    state = s.clone()
                }
                _ => {}
            }
        }
        last
    }

    const DESC: &str = "wpkh([d34db33f/84h/0h/0h]xpub6C.../0/*)#abcd1234";

    #[test]
    fn plain_qr_is_the_material_except_in_bbqr_mode() {
        assert_eq!(
            process("  wpkh(xpub)  ", ScanMode::Auto, &Collector::default()),
            ScanResult::Single("wpkh(xpub)".into())
        );
        assert_eq!(
            process("wpkh(xpub)", ScanMode::Bbqr, &Collector::default()),
            ScanResult::Ignored("Waiting for a BBQR frame.")
        );
        assert_eq!(
            process("   ", ScanMode::Auto, &Collector::default()),
            ScanResult::Ignored("Empty QR frame ignored.")
        );
        assert_eq!(
            process("B$2U0100ABC", ScanMode::Single, &Collector::default()),
            ScanResult::Ignored("BBQR frame seen; switch to Auto or BBQR to collect it.")
        );
    }

    #[test]
    fn base32_and_hex_frames_collect_in_any_order() {
        let mut fs = frames('2', 'U', &b32(DESC.as_bytes()), 3);
        fs.reverse();
        match collect(&fs, ScanMode::Auto) {
            ScanResult::Complete {
                material, progress, ..
            } => {
                assert_eq!(material, DESC);
                assert_eq!(
                    progress,
                    Progress {
                        received: 3,
                        total: 3,
                        file_type: "U".into()
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        let hex: String = DESC.bytes().map(|b| format!("{b:02X}")).collect();
        assert!(
            matches!(collect(&frames('H', 'J', &hex, 2), ScanMode::Bbqr), ScanResult::Complete { material, .. } if material == DESC)
        );
    }

    #[test]
    fn progress_reports_parts_and_a_header_change_restarts() {
        let fs = frames('2', 'U', &b32(DESC.as_bytes()), 3);
        let first = process(&fs[0], ScanMode::Auto, &Collector::default());
        let ScanResult::Progress { state, progress } = first else {
            panic!()
        };
        assert_eq!((progress.received, progress.total), (1, 3));
        let other = frames('H', 'U', "4142", 1);
        // A frame of another set (another encoding) starts over and completes on its own.
        assert!(
            matches!(process(&other[0], ScanMode::Auto, &state), ScanResult::Complete { material, .. } if material == "AB")
        );
    }

    #[test]
    fn zlib_frames_inflate_with_a_small_window() {
        let deflated = miniz_oxide::deflate::compress_to_vec(DESC.as_bytes(), 6);
        assert!(
            matches!(collect(&frames('Z', 'U', &b32(&deflated), 2), ScanMode::Auto), ScanResult::Complete { material, .. } if material == DESC)
        );
    }

    #[test]
    fn only_wallet_material_is_accepted() {
        let hex = |s: &str| s.bytes().map(|b| format!("{b:02X}")).collect::<String>();
        assert!(matches!(
            collect(&frames('H', 'B', &hex(DESC), 1), ScanMode::Auto),
            ScanResult::Complete { .. }
        ));
        assert_eq!(
            collect(&frames('H', 'B', &hex("hello world"), 1), ScanMode::Auto),
            ScanResult::Error("BBQR type B is not a wallet export or descriptor.".into())
        );
        assert_eq!(
            collect(&frames('H', 'P', &hex(DESC), 1), ScanMode::Auto),
            ScanResult::Error("BBQR type P is not a wallet export or descriptor.".into())
        );
        assert_eq!(
            collect(&frames('X', 'U', "ABCD", 1), ScanMode::Auto),
            ScanResult::Error("BBQR encoding X is not supported.".into())
        );
        assert_eq!(
            collect(&frames('H', 'U', "ZZ", 1), ScanMode::Auto),
            ScanResult::Error("Could not decode BBQR HEX payload.".into())
        );
    }
}
