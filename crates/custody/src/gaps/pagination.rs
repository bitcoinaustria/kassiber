//! Pure keyset paging over the complete bounded population. Storage/readiness
//! adapters remain Python-owned. Version and rows are explicit local facts.
use crate::{codec, graph::stored};
use kassiber_pyjson::{BigInt, Value as Py};
use serde_json::{json, Value};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
fn encode(bytes: &[u8]) -> String {
    let mut result = String::new();
    let mut bits = 0u32;
    let mut count = 0;
    for &byte in bytes {
        bits = (bits << 8) | u32::from(byte);
        count += 8;
        while count >= 6 {
            count -= 6;
            result.push(ALPHABET[((bits >> count) & 63) as usize] as char);
        }
    }
    if count > 0 {
        result.push(ALPHABET[((bits << (6 - count)) & 63) as usize] as char);
    }
    result
}
fn decode(text: &str) -> Option<Vec<u8>> {
    // urlsafe_b64decode adds padding based on the original text length and
    // binascii's non-validating decoder ignores other ASCII characters.
    if !text.is_ascii() {
        return None;
    }
    let mut text = text.as_bytes().to_vec();
    text.extend(std::iter::repeat_n(b'=', (4 - text.len() % 4) % 4));
    let mut output = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    let mut padding = 0;
    for byte in text {
        if byte == b'=' {
            if count == 12 || count == 18 {
                padding += 1;
                if count == 18 || padding == 2 {
                    return Some(output);
                }
            }
            continue;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        padding = 0;
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count % 24 == 12 {
            output.push((bits >> 4) as u8);
        }
        if count % 24 == 18 {
            output.push((bits >> 2) as u8);
        }
        if count == 24 {
            output.push(bits as u8);
            count = 0;
        }
    }
    (count == 0).then_some(output)
}
fn json_text(bytes: &[u8]) -> Option<Vec<u8>> {
    // json.loads(bytes) accepts UTF-8, UTF-16 and UTF-32, with or without BOM.
    let (bytes, width, little) = if bytes.starts_with(&[0xff, 0xfe, 0, 0]) {
        (&bytes[4..], 4, true)
    } else if bytes.starts_with(&[0, 0, 0xfe, 0xff]) {
        (&bytes[4..], 4, false)
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        (&bytes[2..], 2, true)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        (&bytes[2..], 2, false)
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        (&bytes[3..], 1, false)
    } else if bytes.len() >= 4 && bytes[..3] == [0, 0, 0] {
        (bytes, 4, false)
    } else if bytes.len() >= 4 && bytes[1..4] == [0, 0, 0] {
        (bytes, 4, true)
    } else if bytes.len() >= 2 && bytes[0] == 0 {
        (bytes, 2, false)
    } else if bytes.len() >= 2 && bytes[1] == 0 {
        (bytes, 2, true)
    } else {
        (bytes, 1, false)
    };
    if width == 1 {
        return Some(bytes.to_vec());
    }
    if bytes.len() % width != 0 {
        return None;
    }
    let points: Vec<u32> = if width == 2 {
        let words = bytes.as_chunks::<2>().0.iter().map(|b| {
            if little {
                u16::from_le_bytes([b[0], b[1]])
            } else {
                u16::from_be_bytes([b[0], b[1]])
            }
        });
        char::decode_utf16(words)
            .map(|point| match point {
                Ok(point) => u32::from(point),
                Err(error) => u32::from(error.unpaired_surrogate()),
            })
            .collect()
    } else {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                if little {
                    u32::from_le_bytes(*b)
                } else {
                    u32::from_be_bytes(*b)
                }
            })
            .collect()
    };
    let mut result = Vec::new();
    for point in points {
        if (0xd800..=0xdfff).contains(&point) {
            result.extend([
                0xe0 | (point >> 12) as u8,
                0x80 | ((point >> 6) & 63) as u8,
                0x80 | (point & 63) as u8,
            ]);
        } else {
            let mut encoded = [0; 4];
            result.extend(char::from_u32(point)?.encode_utf8(&mut encoded).as_bytes());
        }
    }
    Some(result)
}
fn nonnegative(value: &Py) -> bool {
    matches!(value,Py::Int(n) if *n>=BigInt::from(0))
}
pub fn encode_cursor(version: &Value, ordinal: usize, gap_id: &str) -> String {
    let payload = codec::from_json(&json!([version, ordinal, gap_id])).expect("JSON cursor");
    let payload = codec::dumps(&payload, true).expect("JSON cursor");
    format!("cgr3.{}", encode(payload.as_bytes()))
}
pub fn decode_cursor(cursor: &str) -> Result<Py, String> {
    let malformed = || "cursor is malformed".to_owned();
    let encoded = cursor.strip_prefix("cgr3.").ok_or_else(malformed)?;
    let bytes = decode(encoded).ok_or_else(malformed)?;
    let text = json_text(&bytes).ok_or_else(malformed)?;
    let payload = codec::loads_bytes(&text).map_err(|_| malformed())?;
    let Py::List(items) = &payload else {
        return Err(malformed());
    };
    if items.len() != 3
        || !matches!(&items[0],Py::List(v) if v.len()==2 && v.iter().all(nonnegative))
        || !nonnegative(&items[1])
        || !matches!(&items[2], Py::Str(_))
    {
        return Err(malformed());
    }
    Ok(payload)
}
pub fn page_candidates(
    version: &Value,
    population: &[Value],
    limit: usize,
    cursor: Option<&str>,
    gap_id: Option<&str>,
) -> Result<Value, String> {
    if !(1..=500).contains(&limit) {
        return Err("limit must be an integer between 1 and 500".into());
    }
    let cursor = cursor.filter(|v| *v != "0");
    if gap_id.is_some() && cursor.is_some() {
        return Err("cursor is not supported when gap_id is provided".into());
    }
    let mut start = 0;
    if let Some(cursor) = cursor {
        let decoded = decode_cursor(cursor)?;
        let items = stored::list(&decoded);
        if items[0] != codec::from_json(version)? {
            return Err("cursor expired because custody evidence changed".into());
        }
        let Py::Int(ordinal) = &items[1] else {
            unreachable!("validated cursor ordinal")
        };
        let ordinal = ordinal.to_string().parse::<usize>().ok();
        if ordinal.is_none_or(|index| {
            index >= population.len()
                || codec::from_json(&population[index]["gap_id"]).ok().as_ref() != Some(&items[2])
        }) {
            return Err("cursor expired because custody candidates changed".into());
        }
        start = ordinal.unwrap() + 1;
    }
    if let Some(id) = gap_id {
        return Ok(
            json!({"gaps":population.iter().filter(|row|row["gap_id"]==id).take(1).collect::<Vec<_>>(),"next_cursor":null}),
        );
    }
    let page: Vec<_> = population.iter().skip(start).take(limit).collect();
    let next = if start + limit < population.len() && !page.is_empty() {
        Some(encode_cursor(
            version,
            start + page.len() - 1,
            page.last().unwrap()["gap_id"]
                .as_str()
                .expect("candidate id"),
        ))
    } else {
        None
    };
    Ok(json!({"gaps":page,"next_cursor":next}))
}
