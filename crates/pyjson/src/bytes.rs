//! CPython json.loads detects UTF-8/16/32 in bytes, independently of str().

/// Decode JSON bytes with CPython's BOM/zero-byte encoding detection.
/// Malformed encodings are refused; callers own their exception policy.
pub fn json_bytes_string(bytes: &[u8]) -> Result<crate::PyStr, &'static str> {
    let (bytes, width, little) = if bytes.starts_with(b"\xff\xfe\0\0") {
        (&bytes[4..], 4, true)
    } else if bytes.starts_with(b"\0\0\xfe\xff") {
        (&bytes[4..], 4, false)
    } else if bytes.starts_with(b"\xff\xfe") {
        (&bytes[2..], 2, true)
    } else if bytes.starts_with(b"\xfe\xff") {
        (&bytes[2..], 2, false)
    } else if bytes.starts_with(b"\xef\xbb\xbf") {
        (&bytes[3..], 1, false)
    } else if bytes.len() >= 4 && bytes[0] == 0 {
        (bytes, if bytes[1] == 0 { 4 } else { 2 }, false)
    } else if bytes.len() >= 4 && bytes[1] == 0 {
        (
            bytes,
            if bytes[2] == 0 && bytes[3] == 0 { 4 } else { 2 },
            true,
        )
    } else if bytes.len() == 2 && bytes[0] == 0 {
        (bytes, 2, false)
    } else if bytes.len() == 2 && bytes[1] == 0 {
        (bytes, 2, true)
    } else {
        (bytes, 1, false)
    };
    if width == 1 {
        if let Ok(text) = std::str::from_utf8(bytes) {
            return Ok(crate::PyStr::from(text));
        }
        let mut points = Vec::new();
        let mut remaining = bytes;
        while !remaining.is_empty() {
            match std::str::from_utf8(remaining) {
                Ok(text) => {
                    points.extend(text.chars().map(u32::from));
                    break;
                }
                Err(error) => {
                    let end = error.valid_up_to();
                    points.extend(
                        std::str::from_utf8(&remaining[..end])
                            .unwrap()
                            .chars()
                            .map(u32::from),
                    );
                    remaining = &remaining[end..];
                    if remaining.len() < 3
                        || remaining[0] != 0xed
                        || !(0xa0..=0xbf).contains(&remaining[1])
                        || !(0x80..=0xbf).contains(&remaining[2])
                    {
                        return Err("invalid JSON byte encoding");
                    }
                    points.push(
                        (u32::from(remaining[0] & 15) << 12)
                            | (u32::from(remaining[1] & 63) << 6)
                            | u32::from(remaining[2] & 63),
                    );
                    remaining = &remaining[3..];
                }
            }
        }
        return crate::PyStr::from_code_points(points).map_err(|_| "invalid JSON byte encoding");
    }
    if !bytes.len().is_multiple_of(width) {
        return Err("truncated JSON byte encoding");
    }
    let points: Vec<u32> = if width == 2 {
        let units = bytes.as_chunks::<2>().0.iter().map(|p| {
            if little {
                u16::from_le_bytes([p[0], p[1]])
            } else {
                u16::from_be_bytes([p[0], p[1]])
            }
        });
        char::decode_utf16(units)
            .map(|c| {
                c.map(u32::from)
                    .unwrap_or_else(|e| u32::from(e.unpaired_surrogate()))
            })
            .collect()
    } else {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                if little {
                    u32::from_le_bytes(*p)
                } else {
                    u32::from_be_bytes(*p)
                }
            })
            .collect()
    };
    crate::PyStr::from_code_points(points).map_err(|_| "invalid JSON byte encoding")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bytes_repr, dumps, Error, Options, Value};
    #[test]
    fn bytes_json_policy_and_repr_match_python() {
        for (bytes, expected) in [
            (b"".as_slice(), "b''"),
            (b"'", "b\"'\""),
            (b"'\"\\\n\t\r\0\xff", "b'\\\'\"\\\\\\n\\t\\r\\x00\\xff'"),
        ] {
            assert_eq!(bytes_repr(bytes), expected);
            let value = Value::Bytes(bytes.to_vec());
            assert_eq!(
                dumps(&value, &Options::new()),
                Err(Error::BytesNotSerializable)
            );
            assert_eq!(
                dumps(&value, &Options::new().default_str(true)).unwrap(),
                dumps(&Value::from(expected), &Options::new()).unwrap()
            );
        }
    }
    #[test]
    fn json_bytes_detect_encoding_without_changing_bytes_identity() {
        for bytes in [
            b"77".to_vec(),
            b"\xef\xbb\xbf77".to_vec(),
            b"\xff\xfe7\0".to_vec(),
            b"\xfe\xff\0\x37".to_vec(),
            b"\xff\xfe\0\0\x37\0\0\0".to_vec(),
            b"\0\0\xfe\xff\0\0\0\x37".to_vec(),
            b"\0\x37".to_vec(),
            b"7\0".to_vec(),
            b"\0\0\0\x37".to_vec(),
            b"7\0\0\0".to_vec(),
        ] {
            let expected = if bytes.ends_with(b"77") { "77" } else { "7" };
            assert_eq!(json_bytes_string(&bytes).unwrap().as_str(), Some(expected));
        }
        assert!(json_bytes_string(b"\xff").is_err());
        assert!(json_bytes_string(b"\xff\xfe\0").is_err());
        assert!(json_bytes_string(b"\xff\xfe\0\0\0\0\x11\0").is_err());
        for bytes in [
            b"\xff\xfe\0\xd8".as_slice(),
            b"\xfe\xff\xd8\0".as_slice(),
            b"\xff\xfe\0\0\0\xd8\0\0".as_slice(),
            b"\xed\xa0\x80".as_slice(),
        ] {
            assert_eq!(
                json_bytes_string(bytes)
                    .unwrap()
                    .code_points()
                    .collect::<Vec<_>>(),
                vec![0xd800]
            );
        }
        for bytes in [
            b"\xed\xa0".as_slice(),
            b"\xed\x9f\x80\xff".as_slice(),
            b"\xed\xa0\x7f".as_slice(),
        ] {
            assert!(json_bytes_string(bytes).is_err());
        }
    }
}
